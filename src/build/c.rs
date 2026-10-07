//! C and C++ with wit-bindgen and the WASI SDK.

use std::{fs, path::Path, process::Command};

use anyhow::bail;

use super::{Entry, Language, project_directory, run, toolchain};

pub fn build(
    source: &Path,
    entry: &Entry,
    language: Language,
    wit: &Path,
    output: &Path,
) -> anyhow::Result<()> {
    let cpp = language == Language::Cpp;
    let sources = sources(source, cpp)?;
    let generated = tempfile::tempdir()?;
    run(
        Command::new(toolchain::binary("wit-bindgen")?)
            .arg("c")
            .arg(wit)
            .args(["--world", "statespace", "--out-dir"])
            .arg(generated.path()),
        "wit-bindgen c",
    )?;
    // The user function takes and returns NUL-terminated JSON it allocates with malloc.
    let wrapper = generated
        .path()
        .join(if cpp { "wrapper.cpp" } else { "wrapper.c" });
    let linkage = if cpp { "extern \"C\" " } else { "" };
    fs::write(
        &wrapper,
        format!(
            "#include \"statespace.h\"
#include <stdlib.h>
#include <string.h>

{linkage}char *{function}(const char *input);

{linkage}void exports_statespace_execute(statespace_string_t *input, statespace_string_t *ret) {{
    char *text = (char *)malloc(input->len + 1);
    if (text == NULL) __builtin_trap();
    memcpy(text, input->ptr, input->len);
    text[input->len] = '\\0';
    char *result = {function}(text);
    free(text);
    if (result == NULL) __builtin_trap();
    statespace_string_dup(ret, result);
    free(result);
}}
",
            function = entry.function,
        ),
    )?;
    let sdk = toolchain::wasi_sdk()?.join("bin");
    let clang = sdk.join("wasm32-wasip2-clang");
    let bindings = generated.path().join("statespace.o");
    run(
        Command::new(&clang)
            .arg("-c")
            .arg(generated.path().join("statespace.c"))
            .arg("-o")
            .arg(&bindings),
        "wasm32-wasip2-clang",
    )?;
    let compiler = if cpp {
        sdk.join("wasm32-wasip2-clang++")
    } else {
        clang
    };
    let mut command = Command::new(compiler);
    command
        .arg("-mexec-model=reactor")
        .arg("-I")
        .arg(generated.path())
        .arg("-I")
        .arg(project_directory(source)?)
        .args(&sources)
        .arg(&wrapper)
        .arg(&bindings)
        .arg(generated.path().join("statespace_component_type.o"))
        .arg("-o")
        .arg(output);
    run(&mut command, "the WASI compiler")
}

/// The source file, or every C or C++ file in the source directory.
fn sources(source: &Path, cpp: bool) -> anyhow::Result<Vec<std::path::PathBuf>> {
    let matches = |path: &Path| {
        let extension = path.extension().and_then(|value| value.to_str());
        if cpp {
            matches!(extension, Some("cc" | "cpp" | "cxx"))
        } else {
            extension == Some("c")
        }
    };
    let mut sources = if source.is_file() {
        vec![source.to_path_buf()]
    } else {
        let mut files = Vec::new();
        for item in fs::read_dir(source)? {
            let path = item?.path();
            if path.is_file() && matches(&path) {
                files.push(path);
            }
        }
        files
    };
    sources.sort();
    if sources.is_empty() || !sources.iter().all(|path| matches(path)) {
        bail!("the source must contain C or C++ files");
    }
    Ok(sources)
}
