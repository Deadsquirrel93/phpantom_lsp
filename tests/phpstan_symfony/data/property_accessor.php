<?php declare(strict_types = 1);

use function PHPStan\Testing\assertType;

$propertyAccessor = new \Symfony\Component\PropertyAccess\PropertyAccessor();

$array = [1 => 'ea'];
$propertyAccessor->setValue($array, 'foo', 'bar');
assertType('array<mixed>', $array); // SKIP: a @param-out wider than the argument keeps the argument's type

$object = new \stdClass();
$propertyAccessor->setValue($object, 'foo', 'bar');
assertType('stdClass', $object);
