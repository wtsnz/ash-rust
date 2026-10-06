defmodule Absinthe.Phase.Document.Validation.UniqueFragmentNames do
  @moduledoc false

  # Validates document to ensure that all fragments have unique names.

  alias Absinthe.{Blueprint, Phase}

  use Absinthe.Phase

  @doc """
  Run the validation.
  """
  @spec run(Blueprint.t(), Keyword.t()) :: Phase.result_t()
  def run(input, _options \\ []) do
    counts = Enum.frequencies_by(input.fragments, & &1.name)

    fragments =
      Enum.map(input.fragments, fn fragment ->
        if Map.fetch!(counts, fragment.name) > 1 do
          fragment
          |> flag_invalid(:duplicate_name)
          |> put_error(error(fragment))
        else
          fragment
        end
      end)

    {:ok, %{input | fragments: fragments}}
  end

  # Generate an error for a duplicate fragment.
  @spec error(Blueprint.Document.Fragment.Named.t()) :: Phase.Error.t()
  defp error(node) do
    %Phase.Error{
      phase: __MODULE__,
      message: error_message(node.name),
      locations: [node.source_location]
    }
  end

  @doc """
  Generate an error message for a duplicate fragment.
  """
  @spec error_message(String.t()) :: String.t()
  def error_message(name) do
    ~s(There can only be one fragment named "#{name}".)
  end
end
