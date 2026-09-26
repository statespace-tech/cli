use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, bail};
use sha2::{Digest, Sha256};
use wasmparser::{
    ComponentExternalKind, Parser, Payload, PrimitiveValType, Validator,
    component_types::ComponentValType,
};

use crate::{FunctionLanguage, toolchain};

const WIT: &str = "package statespace:component;\nworld statespace {\n  export execute: func(input: string) -> string;\n}\n";
const MAX_ARTIFACT_SIZE: usize = 64 * 1024 * 1024;

pub struct Artifact {
    pub sha256: String,
    pub size: usize,
}

pub fn inspect(path: &Path) -> anyhow::Result<Artifact> {
    let bytes = fs::read(path).with_context(|| format!("could not read {}", path.display()))?;
    if bytes.is_empty() || bytes.len() > MAX_ARTIFACT_SIZE {
        bail!("component size must be 1 byte to 64 MiB");
    }
    if !Parser::is_component(&bytes) {
        bail!("artifact must be a WebAssembly component");
    }
    let types = Validator::new().validate_all(&bytes)?;
    let mut execute_index = None;
    let mut depth = 0_u32;
    for payload in Parser::new(0).parse_all(&bytes) {
        match payload? {
            Payload::ModuleSection { .. } | Payload::ComponentSection { .. } => depth += 1,
            Payload::End(_) => depth = depth.saturating_sub(1),
            Payload::ComponentImportSection(section) if depth == 0 => {
                for import in section {
                    let name = import?.name.name;
                    if !supported_wasi_import(name) {
                        bail!("component imports unsupported host capability: {name}");
                    }
                }
            }
            Payload::ComponentExportSection(section) if depth == 0 => {
                for export in section {
                    let export = export?;
                    if export.name.name == "execute" && export.kind == ComponentExternalKind::Func {
                        execute_index = Some(export.index);
                    }
                }
            }
            _ => {}
        }
    }
    let index = execute_index.context("component must export an execute function")?;
    let function = types
        .as_ref()
        .get(types.component_function_at(index))
        .context("execute function type is unavailable")?;
    if function.async_
        || function.params.len() != 1
        || !matches!(
            function.params[0].1,
            ComponentValType::Primitive(PrimitiveValType::String)
        )
        || !matches!(
            function.result,
            Some(ComponentValType::Primitive(PrimitiveValType::String))
        )
    {
        bail!("execute must have signature func(input: string) -> string");
    }
    Ok(Artifact {
        sha256: format!("{:x}", Sha256::digest(&bytes)),
        size: bytes.len(),
    })
}

fn supported_wasi_import(name: &str) -> bool {
    let Some(interface) = name.strip_suffix("@0.2.12") else {
        return false;
    };
    matches!(
        interface,
        "wasi:cli/environment"
            | "wasi:cli/exit"
            | "wasi:cli/stdin"
            | "wasi:cli/stdout"
            | "wasi:cli/stderr"
            | "wasi:cli/terminal-input"
            | "wasi:cli/terminal-output"
            | "wasi:cli/terminal-stdin"
            | "wasi:cli/terminal-stdout"
            | "wasi:cli/terminal-stderr"
            | "wasi:io/error"
            | "wasi:io/poll"
            | "wasi:io/streams"
            | "wasi:clocks/monotonic-clock"
            | "wasi:clocks/wall-clock"
            | "wasi:random/random"
            | "wasi:filesystem/types"
            | "wasi:filesystem/preopens"
    )
}

pub fn build(
    source: &Path,
    language: FunctionLanguage,
    entry: &str,
    output: &Path,
) -> anyhow::Result<()> {
    let source = source
        .canonicalize()
        .with_context(|| format!("source does not exist: {}", source.display()))?;
    let output = absolute(output)?;
    let parent = output.parent().context("output path has no parent")?;
    fs::create_dir_all(parent)?;
    let temporary = tempfile::tempdir()?;
    let wit = temporary.path().join("statespace.wit");
    fs::write(&wit, WIT)?;
    match language {
        FunctionLanguage::Python => build_python(&source, entry, &wit, &output)?,
        FunctionLanguage::Typescript | FunctionLanguage::Javascript => {
            build_javascript(&source, entry, &wit, &output)?;
        }
        FunctionLanguage::Go => build_go(&source, entry, &wit, &output)?,
        FunctionLanguage::Rust => build_rust(&source, entry, &wit, &output)?,
        FunctionLanguage::C | FunctionLanguage::Cpp => {
            build_c_family(&source, entry, language, &wit, &output)?;
        }
    }
    inspect(&output)?;
    Ok(())
}

fn build_python(source: &Path, entry: &str, wit: &Path, output: &Path) -> anyhow::Result<()> {
    let (module, symbol) = split_entry(entry)?;
    if !module
        .split('.')
        .all(|part| !part.is_empty() && part.bytes().all(identifier_byte))
    {
        bail!("Python entry must use module:function");
    }
    let project = project_directory(source)?;
    let wrapper = tempfile::Builder::new()
        .prefix("_ssp_component_")
        .suffix(".py")
        .tempfile_in(&project)?;
    let wrapper_module = wrapper
        .path()
        .file_stem()
        .and_then(|value| value.to_str())
        .context("invalid wrapper path")?;
    let code = format!(
        "import importlib\nimport json\nimport wit_world\n\n_user = getattr(importlib.import_module({module:?}), {symbol:?})\n\nclass WitWorld(wit_world.WitWorld):\n    def execute(self, input: str) -> str:\n        return json.dumps(_user(json.loads(input)))\n"
    );
    fs::write(wrapper.path(), code)?;
    let mut command = toolchain::python_command()?;
    if std::env::var_os("VIRTUAL_ENV").is_none() && project.join(".venv").is_dir() {
        command.env("VIRTUAL_ENV", project.join(".venv"));
    }
    run_command(
        command
            .current_dir(&project)
            .args(["-d"])
            .arg(wit)
            .args([
                "-w",
                "statespace",
                "componentize",
                "--stub-wasi",
                wrapper_module,
                "-o",
            ])
            .arg(output),
        "componentize-py",
    )
}

fn build_javascript(source: &Path, entry: &str, wit: &Path, output: &Path) -> anyhow::Result<()> {
    let (file, symbol) = split_entry(entry)?;
    let project = project_directory(source)?;
    let entry_path = project.join(file).canonicalize()?;
    if !entry_path.starts_with(&project) {
        bail!("entry file must be inside the source directory");
    }
    let relative = entry_path.strip_prefix(&project)?;
    let import_path = format!("./{}", relative.to_string_lossy().replace('\\', "/"));
    let wrapper = tempfile::Builder::new()
        .prefix("_ssp_component_")
        .suffix(".mjs")
        .tempfile_in(&project)?;
    let code = format!(
        "import {{ {symbol} as userExecute }} from {import_path:?};\nexport function execute(input) {{\n  return JSON.stringify(userExecute(JSON.parse(input)));\n}}\n"
    );
    fs::write(wrapper.path(), code)?;
    run_command(
        toolchain::javascript_command()?
            .current_dir(&project)
            .arg("componentize")
            .arg(wrapper.path())
            .args(["--bundle", "--disable=all", "--wit"])
            .arg(wit)
            .arg("--world-name")
            .arg("statespace")
            .arg("-o")
            .arg(output),
        "jco",
    )
}

fn build_c_family(
    source: &Path,
    entry: &str,
    language: FunctionLanguage,
    wit: &Path,
    output: &Path,
) -> anyhow::Result<()> {
    if entry.is_empty()
        || !entry.bytes().all(identifier_byte)
        || entry.as_bytes()[0].is_ascii_digit()
    {
        bail!("C and C++ entries must be function names");
    }
    let is_cpp = matches!(language, FunctionLanguage::Cpp);
    let project = project_directory(source)?;
    let mut sources = Vec::new();
    if source.is_file() {
        sources.push(source.to_path_buf());
    } else {
        for item in fs::read_dir(source)? {
            let path = item?.path();
            if path.is_file() && c_family_extension(&path, is_cpp) {
                sources.push(path);
            }
        }
        sources.sort();
    }
    if sources.is_empty() || sources.iter().any(|path| !c_family_extension(path, is_cpp)) {
        bail!("source must contain a C or C++ source file");
    }
    let temporary = tempfile::tempdir()?;
    run_command(
        Command::new(toolchain::binary("wit-bindgen", "0.62.0")?)
            .arg("c")
            .arg(wit)
            .args(["--world", "statespace", "--out-dir"])
            .arg(temporary.path()),
        "wit-bindgen c",
    )?;
    let wrapper = temporary
        .path()
        .join(if is_cpp { "wrapper.cpp" } else { "wrapper.c" });
    let exported = if is_cpp { "extern \"C\" void" } else { "void" };
    let code = format!(
        "#include \"statespace.h\"\n#include <stdlib.h>\n#include <string.h>\n\nchar *{entry}(const char *input);\n\n{exported} exports_statespace_execute(statespace_string_t *input, statespace_string_t *ret) {{\n    if (input->len == SIZE_MAX) __builtin_trap();\n    char *text = (char *)malloc(input->len + 1);\n    if (text == NULL) __builtin_trap();\n    memcpy(text, input->ptr, input->len);\n    text[input->len] = '\\0';\n    char *result = {entry}(text);\n    free(text);\n    if (result == NULL) __builtin_trap();\n    statespace_string_dup(ret, result);\n    free(result);\n}}\n"
    );
    fs::write(&wrapper, code)?;
    let c_compiler = wasi_compiler(false)?;
    let cpp_compiler = wasi_compiler(true)?;
    let generated_object = temporary.path().join("statespace.o");
    run_command(
        Command::new(&c_compiler)
            .arg("-c")
            .arg(temporary.path().join("statespace.c"))
            .arg("-o")
            .arg(&generated_object),
        "WASI C compiler",
    )?;
    let compiler = if is_cpp { &cpp_compiler } else { &c_compiler };
    let mut command = Command::new(compiler);
    command
        .arg("-mexec-model=reactor")
        .arg("-I")
        .arg(temporary.path())
        .arg("-I")
        .arg(project);
    for path in sources {
        command.arg(path);
    }
    command
        .arg(wrapper)
        .arg(generated_object)
        .arg(temporary.path().join("statespace_component_type.o"))
        .arg("-o")
        .arg(output);
    run_command(&mut command, "WASI component compiler")
}

fn c_family_extension(path: &Path, is_cpp: bool) -> bool {
    let extension = path.extension().and_then(|value| value.to_str());
    if is_cpp {
        matches!(extension, Some("cpp" | "cc" | "cxx"))
    } else {
        extension == Some("c")
    }
}

fn wasi_compiler(cpp: bool) -> anyhow::Result<PathBuf> {
    let binary = if cpp {
        "wasm32-wasip2-clang++"
    } else {
        "wasm32-wasip2-clang"
    };
    let binary = if cfg!(windows) {
        format!("{binary}.exe")
    } else {
        binary.to_string()
    };
    Ok(toolchain::wasi_sdk()?.join("bin").join(binary))
}

fn build_go(source: &Path, entry: &str, wit: &Path, output: &Path) -> anyhow::Result<()> {
    if !source.is_dir() || !source.join("go.mod").is_file() {
        bail!("Go source must be a directory with go.mod");
    }
    let (package, function) = split_entry(entry)?;
    if !function.as_bytes()[0].is_ascii_uppercase()
        || package.starts_with('/')
        || (package != "."
            && package
                .split('/')
                .any(|part| part.is_empty() || part == "." || part == ".."))
    {
        bail!("Go entry must use relative-package:ExportedFunction");
    }
    let module_output = Command::new("go")
        .current_dir(source)
        .args(["list", "-m"])
        .output()
        .context("could not run go list -m")?;
    if !module_output.status.success() {
        bail!(
            "go list -m failed: {}",
            String::from_utf8_lossy(&module_output.stderr).trim()
        );
    }
    let module = std::str::from_utf8(&module_output.stdout)?.trim();
    if module.is_empty() {
        bail!("Go module path is empty");
    }
    let import_path = if package == "." {
        module.to_string()
    } else {
        format!("{module}/{}", package.trim_start_matches("./"))
    };
    let temporary = tempfile::tempdir()?;
    run_command(
        Command::new(toolchain::binary("componentize-go", "0.4.3")?)
            .current_dir(source)
            .arg("-d")
            .arg(wit)
            .args(["-w", "statespace", "bindings", "--generate-stubs", "-o"])
            .arg(temporary.path()),
        "componentize-go bindings",
    )?;
    let go_mod = temporary.path().join("go.mod");
    let mut manifest = fs::read_to_string(&go_mod)?;
    manifest.push_str(&format!(
        "\nrequire {module} v0.0.0\nreplace {module} => {:?}\n",
        source.to_string_lossy()
    ));
    fs::write(go_mod, manifest)?;
    let code = format!(
        r#"package export_wit_world

import (
    "encoding/json"
    "reflect"
    user {import_path:?}
)

func Execute(input string) string {{
    function := reflect.ValueOf(user.{function})
    signature := function.Type()
    errorType := reflect.TypeOf((*error)(nil)).Elem()
    if signature.NumIn() != 1 || signature.NumOut() < 1 || signature.NumOut() > 2 ||
        (signature.NumOut() == 2 && signature.Out(1) != errorType) {{
        panic("component entry must accept one JSON value and return a value or (value, error)")
    }}
    argument := reflect.New(signature.In(0))
    if err := json.Unmarshal([]byte(input), argument.Interface()); err != nil {{ panic(err) }}
    results := function.Call([]reflect.Value{{argument.Elem()}})
    if len(results) == 2 && !results[1].IsNil() {{ panic(results[1].Interface()) }}
    output, err := json.Marshal(results[0].Interface())
    if err != nil {{ panic(err) }}
    return string(output)
}}
"#
    );
    fs::write(
        temporary.path().join("export_wit_world/wit_bindings.go"),
        code,
    )?;
    run_command(
        Command::new("go")
            .current_dir(temporary.path())
            .args(["mod", "tidy"]),
        "go mod tidy",
    )?;
    run_command(
        Command::new(toolchain::binary("componentize-go", "0.4.3")?)
            .current_dir(temporary.path())
            .arg("-d")
            .arg(wit)
            .args(["-w", "statespace", "build", "-o"])
            .arg(output),
        "componentize-go build",
    )
}

fn build_rust(source: &Path, entry: &str, wit: &Path, output: &Path) -> anyhow::Result<()> {
    if !source.is_dir() || !source.join("Cargo.toml").is_file() {
        bail!("Rust source must be a directory with Cargo.toml");
    }
    if !entry
        .split("::")
        .all(|part| !part.is_empty() && part.bytes().all(identifier_byte))
    {
        bail!("Rust entry must use module::function");
    }
    let manifest = fs::read_to_string(source.join("Cargo.toml"))?;
    let package = manifest
        .lines()
        .find_map(|line| line.trim().strip_prefix("name = "))
        .map(|value| value.trim().trim_matches('"'))
        .context("Cargo.toml has no package name")?;
    let temporary = tempfile::tempdir()?;
    fs::create_dir_all(temporary.path().join("src"))?;
    fs::create_dir_all(temporary.path().join("wit"))?;
    fs::copy(wit, temporary.path().join("wit/world.wit"))?;
    let dependency = source
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('"', "\\\"");
    fs::write(
        temporary.path().join("Cargo.toml"),
        format!(
            "[package]\nname = \"statespace_component\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[lib]\ncrate-type = [\"cdylib\"]\n\n[dependencies]\nserde_json = \"1\"\nwit-bindgen = \"0.49\"\nuser_component = {{ package = {package:?}, path = {dependency:?} }}\n"
        ),
    )?;
    fs::write(
        temporary.path().join("src/lib.rs"),
        format!(
            "wit_bindgen::generate!({{ path: \"wit\", world: \"statespace\" }});\n\nstruct Component;\n\nimpl Guest for Component {{\n    fn execute(input: String) -> String {{\n        let value = serde_json::from_str(&input).expect(\"valid JSON input\");\n        let result = user_component::{entry}(value);\n        serde_json::to_string(&result).expect(\"serializable output\")\n    }}\n}}\n\nexport!(Component);\n"
        ),
    )?;
    toolchain::ensure_rust_target()?;
    run_command(
        Command::new("cargo").current_dir(temporary.path()).args([
            "build",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
        ]),
        "cargo build for wasm32-unknown-unknown",
    )?;
    run_command(
        Command::new(toolchain::binary("wasm-tools", "1.259.0")?)
            .current_dir(temporary.path())
            .args(["component", "new"])
            .arg(
                temporary
                    .path()
                    .join("target/wasm32-unknown-unknown/release/statespace_component.wasm"),
            )
            .arg("-o")
            .arg(output),
        "wasm-tools component new",
    )
}

fn split_entry(entry: &str) -> anyhow::Result<(&str, &str)> {
    let (module, symbol) = entry
        .rsplit_once(':')
        .context("entry must use module:function or file:function")?;
    if module.is_empty() || symbol.is_empty() || !symbol.bytes().all(identifier_byte) {
        bail!("entry contains an invalid function name");
    }
    Ok((module, symbol))
}

fn identifier_byte(value: u8) -> bool {
    value.is_ascii_alphanumeric() || value == b'_'
}

fn project_directory(source: &Path) -> anyhow::Result<PathBuf> {
    if source.is_dir() {
        Ok(source.to_path_buf())
    } else {
        Ok(source
            .parent()
            .context("source has no parent")?
            .to_path_buf())
    }
}

fn absolute(path: &Path) -> anyhow::Result<PathBuf> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()?.join(path))
    }
}

fn run_command(command: &mut Command, name: &str) -> anyhow::Result<()> {
    let output = command
        .output()
        .with_context(|| format!("could not run {name}"))?;
    if !output.status.success() {
        bail!(
            "{name} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}
