//! Python with componentize-py.

use std::{fs, path::Path};

use anyhow::{Context, bail};

use super::{Entry, project_directory, run, toolchain};

pub fn build(source: &Path, entry: &Entry, wit: &Path, output: &Path) -> anyhow::Result<()> {
    let module = match entry.module {
        Some(module) => module.to_owned(),
        None if source.is_file() => source
            .file_stem()
            .and_then(|stem| stem.to_str())
            .context("invalid Python file name")?
            .to_owned(),
        None => bail!("a Python package entry must look like module:function"),
    };
    if !module.split('.').all(super::identifier) {
        bail!("invalid Python module {module:?}");
    }
    let project = project_directory(source)?;
    // componentize-py builds a module; this wrapper adapts JSON strings.
    let wrapper = tempfile::Builder::new()
        .prefix("_statespace_")
        .suffix(".py")
        .tempfile_in(&project)?;
    let wrapper_module = wrapper
        .path()
        .file_stem()
        .and_then(|stem| stem.to_str())
        .context("invalid wrapper path")?
        .to_owned();
    fs::write(
        wrapper.path(),
        format!(
            "import importlib
import json

import wit_world

_function = getattr(importlib.import_module({module:?}), {function:?})


class WitWorld(wit_world.WitWorld):
    def execute(self, input: str) -> str:
        return json.dumps(_function(json.loads(input)))
",
            function = entry.function,
        ),
    )?;
    let mut command = toolchain::componentize_py()?;
    if std::env::var_os("VIRTUAL_ENV").is_none() && project.join(".venv").is_dir() {
        command.env("VIRTUAL_ENV", project.join(".venv"));
    }
    command
        .current_dir(&project)
        .arg("-d")
        .arg(wit)
        .args(["-w", "statespace", "componentize", "--stub-wasi"])
        .arg(&wrapper_module)
        .arg("-o")
        .arg(output);
    let cache = project.join("__pycache__");
    let had_cache = cache.exists();
    let result = run(&mut command, "componentize-py");
    remove_bytecode(&cache, had_cache, &wrapper_module);
    result
}

/// componentize-py imports the source, which writes bytecode beside it. Remove
/// what the build created and leave anything that was already there.
fn remove_bytecode(cache: &Path, existed: bool, wrapper_module: &str) {
    if !existed {
        let _ = fs::remove_dir_all(cache);
        return;
    }
    for entry in fs::read_dir(cache).into_iter().flatten().flatten() {
        if entry
            .file_name()
            .to_string_lossy()
            .starts_with(wrapper_module)
        {
            let _ = fs::remove_file(entry.path());
        }
    }
}
