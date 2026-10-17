//! Integration tests for `@param-immediately-invoked-callable` and
//! `@param-later-invoked-callable`: whether what a callback writes through a
//! by-reference capture is there once the call it was handed to returns,
//! read from the callee wherever it is declared.

use phpantom_lsp::Backend;

use crate::common::{
    create_test_backend, create_test_backend_with_full_stubs, hover_at, hover_text,
};

/// Hover on the last occurrence of `needle` in `content`, opened as
/// `file:///test.php` alongside whatever `backend` has indexed already.
fn hover_last(backend: &Backend, content: &str, needle: &str) -> String {
    let offset = content.rfind(needle).expect("needle present");
    let before = &content[..offset];
    let line = before.matches('\n').count() as u32;
    let character = (offset - before.rfind('\n').map_or(0, |i| i + 1)) as u32;
    let hover =
        hover_at(backend, "file:///test.php", content, line, character).expect("expected hover");
    hover_text(&hover).to_string()
}

/// A function declared in another file that keeps its callback for later
/// may not have run it by the time it returns, so the capture can still
/// hold what it held before the call.
#[test]
fn a_later_invoked_function_in_another_file_widens_the_capture() {
    let backend = create_test_backend();
    backend.update_ast(
        "file:///lib.php",
        r#"<?php
namespace Lib;

/** @param-later-invoked-callable $callback */
function on_shutdown(callable $callback): void {}
"#,
    );
    let content = r#"<?php
namespace App;

use function Lib\on_shutdown;

function run(): void {
    $foo = null;
    on_shutdown(function () use (&$foo) {
        $foo = 1;
    });
    $foo;
}
"#;
    let text = hover_last(&backend, content, "$foo;");
    assert!(
        text.contains("$foo = null|1"),
        "the callback may not have run yet: {text}"
    );
}

/// A function that says nothing runs its callable arguments before it
/// returns, wherever it is declared.
#[test]
fn an_untagged_function_in_another_file_replaces_the_capture() {
    let backend = create_test_backend();
    backend.update_ast(
        "file:///lib.php",
        r#"<?php
namespace Lib;

function each_item(callable $callback): void {}
"#,
    );
    let content = r#"<?php
namespace App;

use function Lib\each_item;

function run(): void {
    $foo = null;
    each_item(function () use (&$foo) {
        $foo = 1;
    });
    $foo;
}
"#;
    let text = hover_last(&backend, content, "$foo;");
    assert!(
        text.contains("$foo = 1\n"),
        "the callback ran before the call returned: {text}"
    );
}

/// A method declared in another file that runs its callback before
/// returning leaves the capture holding only what the callback wrote.
#[test]
fn an_immediately_invoked_method_in_another_file_replaces_the_capture() {
    let backend = create_test_backend();
    backend.update_ast(
        "file:///lib.php",
        r#"<?php
namespace Lib;

class Db {
    /** @param-immediately-invoked-callable $callback */
    public function transaction(callable $callback): void {}

    /** @param-immediately-invoked-callable $callback */
    public static function locked(callable $callback): void {}
}
"#,
    );
    let content = r#"<?php
namespace App;

use Lib\Db;

function run(Db $db): void {
    $foo = null;
    $db->transaction(function () use (&$foo) {
        $foo = 1;
    });
    $foo;
}

function run_static(): void {
    $bar = null;
    Db::locked(function () use (&$bar) {
        $bar = 1;
    });
    $bar;
}
"#;
    let text = hover_last(&backend, content, "$foo;");
    assert!(
        text.contains("$foo = 1\n"),
        "an instance call runs the callback before returning: {text}"
    );
    let text = hover_last(&backend, content, "$bar;");
    assert!(
        text.contains("$bar = 1\n"),
        "a static call runs the callback before returning: {text}"
    );
}

/// A method `$this` inherits from a parent declared in another file is
/// read from the parent's declaration.
#[test]
fn an_inherited_immediately_invoked_method_replaces_the_capture() {
    let backend = create_test_backend();
    backend.update_ast(
        "file:///lib.php",
        r#"<?php
namespace Lib;

class Base {
    /** @param-immediately-invoked-callable $callback */
    public function run(callable $callback): void {}
}
"#,
    );
    let content = r#"<?php
namespace App;

class Service extends \Lib\Base {
    public function go(): void {
        $foo = null;
        $this->run(function () use (&$foo) {
            $foo = 1;
        });
        $foo;
    }
}
"#;
    let text = hover_last(&backend, content, "$foo;");
    assert!(
        text.contains("$foo = 1\n"),
        "the inherited method runs the callback before returning: {text}"
    );
}

/// An override that does not say when it runs the callable keeps the
/// promise of the interface method it implements, declared elsewhere.
#[test]
fn an_override_keeps_the_interface_methods_invocation_tag() {
    let backend = create_test_backend();
    backend.update_ast(
        "file:///contract.php",
        r#"<?php
namespace Lib;

interface Transactional {
    /** @param-immediately-invoked-callable $callback */
    public function transaction(callable $callback): void;
}
"#,
    );
    backend.update_ast(
        "file:///db.php",
        r#"<?php
namespace Lib;

class Db implements Transactional {
    public function transaction(callable $callback): void {
        $callback();
    }
}
"#,
    );
    let content = r#"<?php
namespace App;

function run(\Lib\Db $db): void {
    $foo = null;
    $db->transaction(function () use (&$foo) {
        $foo = 1;
    });
    $foo;
}
"#;
    let text = hover_last(&backend, content, "$foo;");
    assert!(
        text.contains("$foo = 1\n"),
        "the implementation runs the callback before returning: {text}"
    );
}

/// The receiver's class is the one the call reaches, not a class in the
/// calling file that happens to share its short name.
#[test]
fn a_same_named_class_in_the_calling_file_is_not_the_receiver() {
    let backend = create_test_backend();
    backend.update_ast(
        "file:///lib.php",
        r#"<?php
namespace Lib;

class Runner {
    public function run(callable $callback): void {}
}
"#,
    );
    let content = r#"<?php
namespace App;

class Runner {
    /** @param-immediately-invoked-callable $callback */
    public function run(callable $callback): void {}
}

function go(\Lib\Runner $runner): void {
    $foo = null;
    $runner->run(function () use (&$foo) {
        $foo = 1;
    });
    $foo;
}
"#;
    let text = hover_last(&backend, content, "$foo;");
    assert!(
        text.contains("$foo = null|1"),
        "Lib\\Runner::run keeps the callback for later: {text}"
    );
}

/// The builtins PHPStan's stubs tag `@param-later-invoked-callable` keep
/// their callback for later.
#[test]
fn set_error_handler_keeps_its_callback_for_later() {
    let backend = create_test_backend_with_full_stubs();
    let content = r#"<?php
function run(): void {
    $error = null;
    set_error_handler(function (int $errno) use (&$error) {
        $error = $errno;
        return true;
    });
    $error;
}
"#;
    let text = hover_last(&backend, content, "$error;");
    assert!(
        text.contains("$error = null|int"),
        "the handler only runs once an error is raised: {text}"
    );
}
