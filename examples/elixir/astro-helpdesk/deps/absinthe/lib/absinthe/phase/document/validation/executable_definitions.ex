defmodule Absinthe.Phase.Document.Validation.ExecutableDefinitions do
  @moduledoc """
  Implements GraphQL spec §5.1.1 "Executable Definitions": a document
  submitted for execution must contain only OperationDefinition and
  FragmentDefinition. Any TypeSystemDefinition or TypeSystemExtension
  is invalid for execution and is rejected here.

  Runs between Phase.Parse and Phase.Blueprint in the `for_document` pipeline.
  """
  use Absinthe.Phase

  alias Absinthe.{Blueprint, Language, Phase}

  @executable_definitions [Language.OperationDefinition, Language.Fragment]

  @spec run(Blueprint.t(), Keyword.t()) :: Phase.result_t()
  def run(%Blueprint{input: %Language.Document{} = document} = blueprint, options \\ []) do
    case Enum.reject(document.definitions, &(&1.__struct__ in @executable_definitions)) do
      [] ->
        {:ok, blueprint}

      type_definitions ->
        errors =
          Enum.map(type_definitions, fn node ->
            %Phase.Error{
              phase: __MODULE__,
              message: "#{label(node)} is not an executable definition",
              locations: [node.loc]
            }
          end)

        blueprint = update_in(blueprint.execution.validation_errors, &(errors ++ &1))

        case Map.new(options) do
          %{jump_phases: true, result_phase: result_phase} -> {:jump, blueprint, result_phase}
          _ -> {:error, blueprint}
        end
    end
  end

  defp label(node) do
    case node do
      %Language.DirectiveDefinition{name: name} -> "Directive `@#{name}`"
      %Language.EnumTypeDefinition{name: name} -> "Enum `#{name}`"
      %Language.InputObjectTypeDefinition{name: name} -> "Input object `#{name}`"
      %Language.InterfaceTypeDefinition{name: name} -> "Interface `#{name}`"
      %Language.ObjectTypeDefinition{name: name} -> "Type `#{name}`"
      %Language.ScalarTypeDefinition{name: name} -> "Scalar `#{name}`"
      %Language.SchemaDeclaration{} -> "A schema definition"
      %Language.TypeExtensionDefinition{definition: %{name: name}} -> "An extension of `#{name}`"
      %Language.UnionTypeDefinition{name: name} -> "Union `#{name}`"
    end
  end
end
