//! JavaScript and TypeScript with jco and ComponentizeJS.

use std::{fs, path::Path};

use anyhow::{Context, bail};

use super::{Entry, project_directory, run, toolchain};

pub fn build(source: &Path, entry: &Entry, wit: &Path, output: &Path) -> anyhow::Result<()> {
    let project = project_directory(source)?;
    let file = match entry.module {
        Some(file) => project.join(file).canonicalize()?,
        None if source.is_file() => source.to_path_buf(),
        None => bail!("a JavaScript directory entry must look like file:function"),
    };
    let relative = file
        .strip_prefix(&project)
        .context("the entry file must be inside the source directory")?;
    let import = format!("./{}", relative.to_string_lossy().replace('\\', "/"));
    // ComponentizeJS exports a module; this wrapper adapts JSON strings.
    let wrapper = tempfile::Builder::new()
        .prefix("_statespace_")
        .suffix(".mjs")
        .tempfile_in(&project)?;
    fs::write(
        wrapper.path(),
        format!(
            "import {{ {function} as run }} from {import:?};

export function execute(input) {{
  return JSON.stringify(run(JSON.parse(input)));
}}
",
            function = entry.function,
        ),
    )?;
    let mut command = toolchain::jco()?;
    command
        .current_dir(&project)
        .arg("componentize")
        .arg(wrapper.path())
        .args(["--bundle", "--disable=all", "--wit"])
        .arg(wit)
        .args(["--world-name", "statespace", "-o"])
        .arg(output);
    run(&mut command, "jco componentize")
}
