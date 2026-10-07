//! Validate and execute Statespace components.
//!
//! A component exports `execute: func(input: string) -> string`, where both
//! strings are JSON. It may import only the WASI 0.2 interfaces below, which
//! excludes sockets and HTTP. Local execution uses the limits the SDKs use.

use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use serde_json::Value;
use sha2::{Digest, Sha256};
use wasmparser::{
    ComponentExternalKind, Parser, Payload, PrimitiveValType, Validator,
    component_types::ComponentValType,
};
use wasmtime::{
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
    component::{Component, Linker, ResourceTable},
};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

pub const WIT: &str = "package statespace:component;

world statespace {
  export execute: func(input: string) -> string;
}
";

const MAX_COMPONENT_BYTES: usize = 64 * 1024 * 1024;
const MAX_MEMORY_BYTES: usize = 256 * 1024 * 1024;
const WASI_VERSION: &str = "@0.2.12";
const WASI_INTERFACES: [&str; 18] = [
    "wasi:cli/environment",
    "wasi:cli/exit",
    "wasi:cli/stdin",
    "wasi:cli/stdout",
    "wasi:cli/stderr",
    "wasi:cli/terminal-input",
    "wasi:cli/terminal-output",
    "wasi:cli/terminal-stdin",
    "wasi:cli/terminal-stdout",
    "wasi:cli/terminal-stderr",
    "wasi:io/error",
    "wasi:io/poll",
    "wasi:io/streams",
    "wasi:clocks/monotonic-clock",
    "wasi:clocks/wall-clock",
    "wasi:random/random",
    "wasi:filesystem/types",
    "wasi:filesystem/preopens",
];

pub struct Execution {
    pub output: Value,
    pub duration_ms: f64,
}

/// Check the size, imports, and `execute` signature of a component and
/// return its SHA-256.
pub fn inspect(bytes: &[u8]) -> anyhow::Result<String> {
    if bytes.is_empty() || bytes.len() > MAX_COMPONENT_BYTES {
        bail!("a component must be 1 byte to 64 MiB");
    }
    if !Parser::is_component(bytes) {
        bail!("the artifact is not a WebAssembly component");
    }
    let types = Validator::new().validate_all(bytes)?;
    let mut execute = None;
    let mut depth = 0_u32;
    for payload in Parser::new(0).parse_all(bytes) {
        match payload? {
            Payload::ModuleSection { .. } | Payload::ComponentSection { .. } => depth += 1,
            Payload::End(_) => depth = depth.saturating_sub(1),
            Payload::ComponentImportSection(imports) if depth == 0 => {
                for import in imports {
                    let name = import?.name.name;
                    let allowed = name
                        .strip_suffix(WASI_VERSION)
                        .is_some_and(|interface| WASI_INTERFACES.contains(&interface));
                    if !allowed {
                        bail!("the component imports an unsupported capability: {name}");
                    }
                }
            }
            Payload::ComponentExportSection(exports) if depth == 0 => {
                for export in exports {
                    let export = export?;
                    if export.name.name == "execute" && export.kind == ComponentExternalKind::Func {
                        execute = Some(export.index);
                    }
                }
            }
            _ => {}
        }
    }
    let index = execute.context("the component does not export an execute function")?;
    let function = types
        .as_ref()
        .get(types.component_function_at(index))
        .context("the execute function type is unavailable")?;
    let string = |value: &ComponentValType| {
        matches!(value, ComponentValType::Primitive(PrimitiveValType::String))
    };
    let valid = !function.async_
        && function.params.len() == 1
        && string(&function.params[0].1)
        && function.result.as_ref().is_some_and(string);
    if !valid {
        bail!("execute must have the signature func(input: string) -> string");
    }
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

struct Host {
    wasi: WasiCtx,
    table: ResourceTable,
    limits: StoreLimits,
}

impl WasiView for Host {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

/// Compile a component, reusing a cached compilation of the same bytes.
fn compile(engine: &Engine, bytes: &[u8]) -> anyhow::Result<Component> {
    let sha256 = format!("{:x}", Sha256::digest(bytes));
    let cache = crate::build::compiled_path(&sha256)?;
    if cache.is_file() {
        // SAFETY: the file was written by `Component::serialize` below with
        // this Wasmtime version and configuration, and lives in the user's
        // private cache directory.
        if let Ok(component) = unsafe { Component::deserialize_file(engine, &cache) } {
            return Ok(component);
        }
    }
    let component = Component::new(engine, bytes)?;
    if let Some(directory) = cache.parent() {
        std::fs::create_dir_all(directory)?;
        let staged = tempfile::NamedTempFile::new_in(directory)?;
        std::fs::write(staged.path(), component.serialize()?)?;
        staged.persist(&cache)?;
    }
    Ok(component)
}

/// Run `execute` once with no filesystem, network, or environment access.
pub fn execute(bytes: &[u8], input: &Value, timeout: Duration) -> anyhow::Result<Execution> {
    inspect(bytes)?;
    let mut config = Config::new();
    config.epoch_interruption(true);
    let engine = Engine::new(&config)?;
    let component = compile(&engine, bytes)?;
    let mut linker = Linker::<Host>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_sync(&mut linker)?;
    let host = Host {
        wasi: WasiCtx::builder().build(),
        table: ResourceTable::new(),
        limits: StoreLimitsBuilder::new()
            .memory_size(MAX_MEMORY_BYTES)
            .build(),
    };
    let mut store = Store::new(&engine, host);
    store.limiter(|host| &mut host.limits);
    store.set_epoch_deadline(1);

    // Interrupt the guest once the timeout elapses.
    let finished = Arc::new(AtomicBool::new(false));
    let watchdog = {
        let (engine, finished) = (engine.clone(), finished.clone());
        thread::spawn(move || {
            let deadline = Instant::now() + timeout;
            while !finished.load(Ordering::Relaxed) {
                if Instant::now() >= deadline {
                    engine.increment_epoch();
                    return;
                }
                thread::sleep(Duration::from_millis(5));
            }
        })
    };
    let started = Instant::now();
    let result = (|| {
        let instance = linker.instantiate(&mut store, &component)?;
        let function = instance.get_typed_func::<(String,), (String,)>(&mut store, "execute")?;
        let (output,) = function.call(&mut store, (serde_json::to_string(input)?,))?;
        anyhow::Ok(output)
    })();
    let elapsed = started.elapsed();
    finished.store(true, Ordering::Relaxed);
    watchdog.join().ok();
    let output = match result {
        Ok(output) => output,
        Err(_) if elapsed >= timeout => bail!("the function exceeded its {timeout:?} timeout"),
        Err(error) => return Err(error.context("the function failed")),
    };
    Ok(Execution {
        output: serde_json::from_str(&output).context("the function returned invalid JSON")?,
        duration_ms: (elapsed.as_secs_f64() * 1e6).round() / 1e3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const IDENTITY: &[u8] = include_bytes!("../tests/fixtures/identity.wasm");

    #[test]
    fn executes_components_in_the_sandbox() {
        let input = serde_json::json!({"items": [3, 1, 2]});
        let execution = execute(IDENTITY, &input, Duration::from_secs(5)).unwrap();
        assert_eq!(execution.output, input);
    }

    #[test]
    fn rejects_files_that_are_not_components() {
        assert!(inspect(b"not a component").is_err());
        assert_eq!(inspect(IDENTITY).unwrap().len(), 64);
    }
}
