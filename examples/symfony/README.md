# Symfony Demo Project for PHPantom LSP

A standalone editor playground for PHPantom's Symfony and Doctrine support. It
uses attributes, YAML, XML, PHP configurators, Twig, and translation
catalogues together, the way a real Symfony project does.

## What to try

- Open `config/services.yaml`, `config/routes.yaml`,
  `config/doctrine/User.orm.xml`, or `config/validator/User.yaml` and use
  go-to-definition on a PHP class name, or on the `Class::method` in a route's
  `controller`.
- Use find references on a class or method mentioned there, such as
  `DemoController::legacy()`. The YAML and XML occurrences are included.
- Rename `legacy` in `config/routes.yaml`, or rename `UserRepository` from
  `config/doctrine/User.orm.xml`. The PHP declaration and every configuration
  file that names it are renamed together.
- Open `src/Entity/User.php` and `src/Repository/UserRepository.php`. The
  CodeLens above each class links the entity and its repository.

## Getting started

1. Run `composer install` in this directory to install Symfony and Doctrine.
2. Open this directory as a project or workspace folder in your editor.

The files are a language-server playground, not a bootable Symfony
application, so no kernel or database is required.
