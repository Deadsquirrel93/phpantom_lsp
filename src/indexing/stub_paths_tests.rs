use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::Backend;
use crate::config::{ClientStubOptions, Config, StubsConfig};

/// A backend over `root` whose `[stubs] paths` are `paths`.
fn backend_with_stub_paths(root: &Path, paths: &[&str]) -> Backend {
    let backend = Backend::new_test_with_workspace(root.to_path_buf(), Vec::new());
    set_stub_paths(&backend, paths);
    backend
}

fn set_stub_paths(backend: &Backend, paths: &[&str]) {
    backend.set_config(Config {
        stubs: StubsConfig {
            paths: Some(paths.iter().map(|p| p.to_string()).collect()),
        },
        ..Config::default()
    });
}

fn write(root: &Path, relative: &str, content: &str) -> std::path::PathBuf {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, content).unwrap();
    path
}

/// Parse a project file the way opening it in the editor would.
fn parse_project_file(backend: &Backend, path: &Path) {
    let content = fs::read_to_string(path).unwrap();
    backend.update_ast(&crate::util::path_to_uri(path), &content);
}

fn method_names(backend: &Backend, class: &str) -> Vec<String> {
    let class = backend
        .find_or_load_class(class)
        .unwrap_or_else(|| panic!("{class} should resolve"));
    class.methods.iter().map(|m| m.name.to_string()).collect()
}

const PROJECT_FOO: &str =
    "<?php\nnamespace App;\nclass Foo { public function fromProject(): void {} }\n";
const STUB_FOO: &str = "<?php\nnamespace App;\nclass Foo { public function fromStub(): void {} }\n";

#[test]
fn a_stub_class_wins_over_the_project_class_of_the_same_name() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let project = write(root, "src/Foo.php", PROJECT_FOO);
    write(root, "stubs/Foo.php", STUB_FOO);
    let backend = backend_with_stub_paths(root, &["stubs"]);

    // Parsed before the stub index exists, as a file opened during
    // startup would be.
    parse_project_file(&backend, &project);
    backend.refresh_stub_paths();

    assert_eq!(method_names(&backend, "App\\Foo"), ["fromStub"]);
}

#[test]
fn removing_the_stub_path_brings_the_project_class_back() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let project = write(root, "src/Foo.php", PROJECT_FOO);
    write(root, "stubs/Foo.php", STUB_FOO);
    let backend = backend_with_stub_paths(root, &["stubs"]);
    parse_project_file(&backend, &project);
    backend.refresh_stub_paths();
    assert_eq!(method_names(&backend, "App\\Foo"), ["fromStub"]);

    set_stub_paths(&backend, &[]);
    backend.refresh_stub_paths();

    assert_eq!(method_names(&backend, "App\\Foo"), ["fromProject"]);
    assert!(!backend.is_external_stub_uri(&crate::util::path_to_uri(&root.join("stubs/Foo.php"))));
}

#[test]
fn a_stub_function_and_class_win_over_the_embedded_builtins() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "stubs/builtins.php",
        "<?php\nfunction strlen(string $string): string {}\nclass ArrayObject { public function fromStub(): void {} }\n",
    );
    let mut backend = Backend::new_test_with_all_stubs(
        HashMap::from([(
            "ArrayObject",
            "<?php\nclass ArrayObject { public function fromEmbedded(): void {} }\n",
        )]),
        HashMap::from([("strlen", "<?php\nfunction strlen(string $string): int {}\n")]),
        HashMap::new(),
    );
    *backend.state_mut().workspace.workspace_root.write() = Some(root.to_path_buf());
    set_stub_paths(&backend, &["stubs"]);

    // The embedded copies are already loaded, as they would be in any
    // session that reached them before the stub paths changed.
    let embedded = backend.find_or_load_function(&["strlen"]).unwrap();
    assert_eq!(embedded.return_type_str().as_deref(), Some("int"));
    assert_eq!(method_names(&backend, "ArrayObject"), ["fromEmbedded"]);

    backend.refresh_stub_paths();

    let function = backend.find_or_load_function(&["strlen"]).unwrap();
    assert_eq!(function.return_type_str().as_deref(), Some("string"));
    assert_eq!(method_names(&backend, "ArrayObject"), ["fromStub"]);
    assert_eq!(
        backend.load_stub_class("ArrayObject").map(|c| c
            .methods
            .iter()
            .map(|m| m.name.to_string())
            .collect::<Vec<_>>()),
        Some(vec!["fromStub".to_string()])
    );
}

#[test]
fn a_stub_constant_wins_over_a_project_define() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let project = write(
        root,
        "src/constants.php",
        "<?php\ndefine('GREETING', 'project');\n",
    );
    write(
        root,
        "stubs/constants.php",
        "<?php\nconst GREETING = 'stub';\n",
    );
    let backend = backend_with_stub_paths(root, &["stubs"]);
    backend.refresh_stub_paths();

    assert_eq!(
        backend.lookup_global_constant("GREETING"),
        Some(Some("'stub'".to_string()))
    );

    // A project file parsed afterwards does not take the name back.
    parse_project_file(&backend, &project);
    assert_eq!(
        backend.lookup_global_constant("GREETING"),
        Some(Some("'stub'".to_string()))
    );
}

#[test]
fn the_config_paths_win_over_the_editor_directory() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "stubs/Shared.php", STUB_FOO);
    let bundled = tempfile::tempdir().unwrap();
    write(bundled.path(), "Shared.php", PROJECT_FOO);
    write(
        bundled.path(),
        "Bundled.php",
        "<?php\nnamespace App;\nclass Bundled {}\n",
    );
    let backend = backend_with_stub_paths(root, &["stubs"]);
    backend.set_client_stub_options(ClientStubOptions {
        path: Some(bundled.path().to_path_buf()),
    });
    backend.refresh_stub_paths();

    assert_eq!(method_names(&backend, "App\\Foo"), ["fromStub"]);
    assert!(backend.find_or_load_class("App\\Bundled").is_some());
}

#[test]
fn a_phpstorm_stubs_checkout_is_read_from_its_map() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let bundled = tempfile::tempdir().unwrap();
    write(
        bundled.path(),
        "PhpStormStubsMap.php",
        "<?php\nnamespace JetBrains\\PHPStormStub;\nfinal class PhpStormStubsMap\n{\nconst CLASSES = array (\n  'Mapped' => 'ext/Mapped.php',\n);\n\nconst FUNCTIONS = array (\n  'mapped_fn' => 'ext/Mapped.php',\n);\n\nconst CONSTANTS = array (\n);\n}\n",
    );
    write(
        bundled.path(),
        "ext/Mapped.php",
        "<?php\nclass Mapped {}\nfunction mapped_fn(): int {}\n",
    );
    // Not in the map, so not a stub: the map is the whole index.
    write(
        bundled.path(),
        "ext/Unmapped.php",
        "<?php\nclass Unmapped {}\n",
    );
    let backend = Backend::new_test_with_workspace(root.to_path_buf(), Vec::new());
    backend.set_client_stub_options(ClientStubOptions {
        path: Some(bundled.path().to_path_buf()),
    });
    backend.refresh_stub_paths();

    assert!(backend.find_or_load_class("Mapped").is_some());
    assert!(backend.find_or_load_function(&["mapped_fn"]).is_some());
    assert!(backend.find_or_load_class("Unmapped").is_none());
}

#[test]
fn a_stub_path_is_registered_as_library_code() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let stub = write(root, "stubs/Foo.php", STUB_FOO);
    let backend = backend_with_stub_paths(root, &["stubs"]);
    backend.refresh_stub_paths();

    let uri = crate::util::path_to_uri(&stub);
    assert!(backend.is_external_stub_uri(&uri));
    assert!(
        backend
            .workspace
            .vendor_uri_prefixes
            .lock()
            .iter()
            .any(|prefix| uri.starts_with(prefix.as_str())),
        "a stub file inside the workspace is still never analysed"
    );
}

#[test]
fn client_stub_options_are_read_from_either_shape() {
    let expected = Some(ClientStubOptions {
        path: Some("/opt/stubs".into()),
    });
    assert_eq!(
        ClientStubOptions::from_client_settings(&serde_json::json!({
            "stubs": { "path": "/opt/stubs" }
        })),
        expected
    );
    assert_eq!(
        ClientStubOptions::from_client_settings(&serde_json::json!({
            "phpantom": { "stubs": { "path": "/opt/stubs" } }
        })),
        expected
    );
    assert_eq!(
        ClientStubOptions::from_client_settings(&serde_json::json!({ "indexing": {} })),
        None
    );
}
