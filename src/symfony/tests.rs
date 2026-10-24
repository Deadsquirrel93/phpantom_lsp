use std::sync::Arc;

use crate::symbol_map::StringKeyKind;

/// A backend whose workspace holds `files`, every one published the way
/// the workspace index publishes it.
fn backend_with(files: &[(&str, &str)]) -> (crate::Backend, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let backend = crate::Backend::new_test_with_workspace(dir.path().to_path_buf(), vec![]);
    for (path, content) in files {
        let full = dir.path().join(path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, content).unwrap();
        let uri = crate::util::path_to_uri(&full);
        backend
            .open_files
            .write()
            .insert(uri.clone(), Arc::new(content.to_string()));
        if crate::resource_navigation::is_resource_document(&uri) {
            backend.update_resource_symbol_index(&uri, content);
        } else {
            backend.update_ast(&uri, content);
        }
    }
    (backend, dir)
}

#[test]
fn declarations_are_read_from_yaml_xml_and_php_configurators() {
    let (backend, _dir) = backend_with(&[
        (
            "config/services.yaml",
            "parameters:\n    app.sender: x\nservices:\n    app.mailer: ~\n",
        ),
        (
            "config/services.xml",
            "<container><services><service id=\"app.xml_mailer\"/></services></container>\n",
        ),
        (
            "config/services.php",
            "<?php\nnamespace Symfony\\Component\\DependencyInjection\\Loader\\Configurator;\n\nreturn static function (ContainerConfigurator $container): void {\n    $container->services()->set('app.php_mailer');\n    $container->parameters()->set('app.php_sender', 'x');\n};\n",
        ),
    ]);

    // The configurator calls are confirmed by the type engine, which needs
    // the configurator classes; without them only the configuration files
    // declare anything.
    assert_eq!(
        backend.symfony_declared_names(StringKeyKind::ServiceId),
        vec!["app.mailer".to_string(), "app.xml_mailer".to_string()]
    );
    assert_eq!(
        backend.symfony_declared_names(StringKeyKind::ContainerParameter),
        vec!["app.sender".to_string()]
    );
}

#[test]
fn a_file_that_stops_declaring_is_forgotten() {
    let (backend, dir) =
        backend_with(&[("config/services.yaml", "services:\n    app.mailer: ~\n")]);
    let uri = crate::util::path_to_uri(&dir.path().join("config/services.yaml"));
    backend.update_resource_symbol_index(&uri, "services:\n");
    assert!(
        backend
            .symfony_declared_names(StringKeyKind::ServiceId)
            .is_empty()
    );
}

#[test]
fn nothing_is_read_outside_a_symfony_project() {
    let dir = tempfile::tempdir().unwrap();
    let backend = crate::Backend::new_test_with_workspace(dir.path().to_path_buf(), vec![]);
    backend.set_is_symfony(false);
    let uri = crate::util::path_to_uri(&dir.path().join("config/services.yaml"));
    backend.update_resource_symbol_index(&uri, "services:\n    app.mailer: ~\n");
    assert!(
        backend
            .symfony_declared_names(StringKeyKind::ServiceId)
            .is_empty()
    );
}
