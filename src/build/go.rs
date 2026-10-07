//! Go with componentize-go.

use std::{fs, path::Path, process::Command};

use anyhow::bail;

use super::{Entry, run, toolchain};

pub fn build(source: &Path, entry: &Entry, wit: &Path, output: &Path) -> anyhow::Result<()> {
    if !source.join("go.mod").is_file() {
        bail!("a Go source must be a module directory with go.mod");
    }
    let package = entry.module.unwrap_or(".");
    let relative = package == "."
        || package
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..");
    if !relative || !entry.function.starts_with(|c: char| c.is_ascii_uppercase()) {
        bail!("a Go entry must look like relative/package:ExportedFunction");
    }
    let listed = Command::new("go")
        .current_dir(source)
        .args(["list", "-m"])
        .output()?;
    if !listed.status.success() {
        bail!(
            "go list -m failed: {}",
            String::from_utf8_lossy(&listed.stderr).trim()
        );
    }
    let module = String::from_utf8(listed.stdout)?.trim().to_owned();
    let import = match package {
        "." => module.clone(),
        package => format!("{module}/{}", package.trim_start_matches("./")),
    };

    let workspace = tempfile::tempdir()?;
    run(
        Command::new(toolchain::binary("componentize-go")?)
            .current_dir(source)
            .arg("-d")
            .arg(wit)
            .args(["-w", "statespace", "bindings", "--generate-stubs", "-o"])
            .arg(workspace.path()),
        "componentize-go bindings",
    )?;
    let manifest = workspace.path().join("go.mod");
    let mut contents = fs::read_to_string(&manifest)?;
    contents.push_str(&format!(
        "\nrequire {module} v0.0.0\nreplace {module} => {:?}\n",
        source.to_string_lossy()
    ));
    fs::write(&manifest, contents)?;
    // Decode the input into the function's parameter type and encode its result.
    fs::write(
        workspace.path().join("export_wit_world/wit_bindings.go"),
        format!(
            r#"package export_wit_world

import (
	"encoding/json"
	"reflect"

	user {import:?}
)

func Execute(input string) string {{
	function := reflect.ValueOf(user.{function})
	signature := function.Type()
	errorType := reflect.TypeOf((*error)(nil)).Elem()
	if signature.NumIn() != 1 || signature.NumOut() < 1 || signature.NumOut() > 2 ||
		(signature.NumOut() == 2 && signature.Out(1) != errorType) {{
		panic("the entry must take one value and return a value or (value, error)")
	}}
	argument := reflect.New(signature.In(0))
	if err := json.Unmarshal([]byte(input), argument.Interface()); err != nil {{
		panic(err)
	}}
	results := function.Call([]reflect.Value{{argument.Elem()}})
	if len(results) == 2 && !results[1].IsNil() {{
		panic(results[1].Interface())
	}}
	output, err := json.Marshal(results[0].Interface())
	if err != nil {{
		panic(err)
	}}
	return string(output)
}}
"#,
            function = entry.function,
        ),
    )?;
    run(
        Command::new("go")
            .current_dir(workspace.path())
            .args(["mod", "tidy"]),
        "go mod tidy",
    )?;
    run(
        Command::new(toolchain::binary("componentize-go")?)
            .current_dir(workspace.path())
            .arg("-d")
            .arg(wit)
            .args(["-w", "statespace", "build", "-o"])
            .arg(output),
        "componentize-go build",
    )
}
