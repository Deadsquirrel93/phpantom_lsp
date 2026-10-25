<?php declare(strict_types = 1);

use Symfony\Component\Config\Definition\Builder\TreeBuilder;
use function PHPStan\Testing\assertType;

// Symfony 7.4 declares the tree builder and the node definitions as
// generics, each node naming its parent, so the types below are the ones
// those declarations give. Upstream expects the bare classes the
// extension resolves for older Symfony versions.

$treeBuilder = new TreeBuilder('my_tree');
$treeRootNode = $treeBuilder->getRootNode();
assertType("Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>", $treeRootNode);

assertType("Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>", $treeRootNode // SKIP: a class template in @return gives way to a scalar or nullable native return type
	->children()
	->end());

assertType("Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>", $treeRootNode // SKIP: a class template in @return gives way to a scalar or nullable native return type
	->children()
	->scalarNode("protocol")
	->end()
	->end());

assertType("Symfony\Component\Config\Definition\Builder\NodeBuilder<Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>>", $treeRootNode
	->children());

assertType("Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\NodeBuilder<Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>>>", $treeRootNode
	->children()
	->arrayNode("protocols"));

assertType("Symfony\Component\Config\Definition\Builder\NodeBuilder<Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\NodeBuilder<Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>>>>", $treeRootNode
	->children()
	->arrayNode("protocols")
	->children());

assertType("Symfony\Component\Config\Definition\Builder\ScalarNodeDefinition<Symfony\Component\Config\Definition\Builder\NodeBuilder<Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\NodeBuilder<Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>>>>>", $treeRootNode
	->children()
	->arrayNode("protocols")
	->children()
	->scalarNode("protocol"));

assertType("Symfony\Component\Config\Definition\Builder\NodeBuilder<Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\NodeBuilder<Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>>>>", $treeRootNode // SKIP: a class template in @return gives way to a scalar or nullable native return type
	->children()
	->arrayNode("protocols")
	->children()
	->scalarNode("protocol")
	->end());

assertType("Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\NodeBuilder<Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>>>", $treeRootNode // SKIP: a class template in @return gives way to a scalar or nullable native return type
	->children()
	->arrayNode("protocols")
	->children()
	->scalarNode("protocol")
	->end()
	->end());

assertType("Symfony\Component\Config\Definition\Builder\NodeBuilder<Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>>", $treeRootNode // SKIP: a class template in @return gives way to a scalar or nullable native return type
	->children()
	->arrayNode("protocols")
	->children()
	->scalarNode("protocol")
	->end()
	->end()
	->end());

assertType("Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>", $treeRootNode // SKIP: a class template in @return gives way to a scalar or nullable native return type
	->children()
	->arrayNode("protocols")
	->children()
	->scalarNode("protocol")
	->end()
	->end()
	->end()
	->end());

assertType("Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>", $treeRootNode // SKIP: a class template in @return gives way to a scalar or nullable native return type
	->children()
	->arrayNode("protocols")
	->children()
	->booleanNode("auto_connect")
	->defaultTrue()
	->end()
	->scalarNode("default_connection")
	->defaultValue("default")
	->end()
	->integerNode("positive_value")
	->min(0)
	->end()
	->floatNode("big_value")
	->max(5E45)
	->end()
	->enumNode("delivery")
	->values(["standard", "expedited", "priority"])
	->end()
	->end()
	->end()
	->end());

$arrayTreeBuilder = new TreeBuilder('my_tree', 'array');
$arrayRootNode = $arrayTreeBuilder->getRootNode();

assertType("Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>", $arrayRootNode);
assertType("Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>", $arrayRootNode->end()); // SKIP: a class template in @return gives way to a scalar or nullable native return type
assertType("Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>", $arrayRootNode // SKIP: a class template in @return gives way to a scalar or nullable native return type
	->children()
	->arrayNode("methods")
	->prototype("scalar")
	->defaultNull()
	->end()
	->end()
	->end());

assertType("Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>", $arrayRootNode // SKIP: a class template in @return gives way to a scalar or nullable native return type
	->children()
	->arrayNode("methods")
	->scalarPrototype()
	->defaultNull()
	->end()
	->end()
	->end());

assertType("Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>", $arrayRootNode // SKIP: a class template in @return gives way to a scalar or nullable native return type
	->children()
	->arrayNode("methods")
	->prototype("scalar")
	->validate()
	->ifNotInArray(["one", "two"])
	->thenInvalid("%s is not a valid method.")
	->end()
	->end()
	->end()
	->end());

assertType("Symfony\Component\Config\Definition\Builder\ArrayNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'array'>>", $arrayRootNode // SKIP: a class template in @return gives way to a scalar or nullable native return type
	->children()
	->arrayNode("methods")
	->prototype("array")
	->beforeNormalization()
	->ifString()
	->then(static function ($v) {
		return [$v];
	})
	->end()
	->end()
	->end()
	->end());

$variableTreeBuilder = new TreeBuilder('my_tree', 'variable');
$variableRootNode = $variableTreeBuilder->getRootNode();

assertType("Symfony\Component\Config\Definition\Builder\VariableNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'variable'>>", $variableRootNode);
assertType("Symfony\Component\Config\Definition\Builder\TreeBuilder<'variable'>", $variableRootNode->end()); // SKIP: a class template in @return gives way to a scalar or nullable native return type

$scalarTreeBuilder = new TreeBuilder('my_tree', 'scalar');
$scalarRootNode = $scalarTreeBuilder->getRootNode();

assertType("Symfony\Component\Config\Definition\Builder\ScalarNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'scalar'>>", $scalarRootNode);
assertType("Symfony\Component\Config\Definition\Builder\ScalarNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'scalar'>>", $scalarRootNode->defaultValue("default"));
assertType("Symfony\Component\Config\Definition\Builder\TreeBuilder<'scalar'>", $scalarRootNode->defaultValue("default")->end()); // SKIP: a class template in @return gives way to a scalar or nullable native return type

$booleanTreeBuilder = new TreeBuilder('my_tree', 'boolean');
$booleanRootNode = $booleanTreeBuilder->getRootNode();

assertType("Symfony\Component\Config\Definition\Builder\BooleanNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'boolean'>>", $booleanRootNode);
assertType("Symfony\Component\Config\Definition\Builder\BooleanNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'boolean'>>", $booleanRootNode->defaultTrue());
assertType("Symfony\Component\Config\Definition\Builder\TreeBuilder<'boolean'>", $booleanRootNode->defaultTrue()->end()); // SKIP: a class template in @return gives way to a scalar or nullable native return type

$integerTreeBuilder = new TreeBuilder('my_tree', 'integer');
$integerRootNode = $integerTreeBuilder->getRootNode();

assertType("Symfony\Component\Config\Definition\Builder\IntegerNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'integer'>>", $integerRootNode);
assertType("Symfony\Component\Config\Definition\Builder\IntegerNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'integer'>>", $integerRootNode->min(0));
assertType("Symfony\Component\Config\Definition\Builder\TreeBuilder<'integer'>", $integerRootNode->min(0)->end()); // SKIP: a class template in @return gives way to a scalar or nullable native return type

$floatTreeBuilder = new TreeBuilder('my_tree', 'float');
$floatRootNode = $floatTreeBuilder->getRootNode();

assertType("Symfony\Component\Config\Definition\Builder\FloatNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'float'>>", $floatRootNode);
assertType("Symfony\Component\Config\Definition\Builder\FloatNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'float'>>", $floatRootNode->max(5E45));
assertType("Symfony\Component\Config\Definition\Builder\TreeBuilder<'float'>", $floatRootNode->max(5E45)->end()); // SKIP: a class template in @return gives way to a scalar or nullable native return type

$enumTreeBuilder = new TreeBuilder('my_tree', 'enum');
$enumRootNode = $enumTreeBuilder->getRootNode();

assertType("Symfony\Component\Config\Definition\Builder\EnumNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'enum'>>", $enumRootNode);
assertType("Symfony\Component\Config\Definition\Builder\EnumNodeDefinition<Symfony\Component\Config\Definition\Builder\TreeBuilder<'enum'>>", $enumRootNode->values(["standard", "expedited", "priority"]));
assertType("Symfony\Component\Config\Definition\Builder\TreeBuilder<'enum'>", $enumRootNode->values(["standard", "expedited", "priority"])->end()); // SKIP: a class template in @return gives way to a scalar or nullable native return type
