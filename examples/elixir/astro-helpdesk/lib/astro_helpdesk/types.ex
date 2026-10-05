defmodule AstroHelpdesk.Types.TicketStatus do
  @moduledoc "Where a ticket stands. GraphQL serves them as `OPEN`, `IN_PROGRESS`, `RESOLVED`, `CLOSED`."
  use Ash.Type.Enum, values: [:open, :in_progress, :resolved, :closed]

  def graphql_type(_), do: :ticket_status
end

defmodule AstroHelpdesk.Types.Attachment do
  @moduledoc """
  Bytes as standard base64 text, which is how the Rust desk serves its `Binary` over GraphQL
  (`String`). AshGraphql has no GraphQL type for `:binary`, so the desk keeps the text, and
  refuses text that isn't base64.
  """
  use Ash.Type.NewType, subtype_of: :string

  def graphql_type(_), do: :string
  def graphql_input_type(_), do: :string

  @impl true
  def cast_input(nil, _constraints), do: {:ok, nil}

  def cast_input(value, _constraints) when is_binary(value) do
    case Base.decode64(value) do
      {:ok, _bytes} -> {:ok, value}
      :error -> {:error, "is not valid base64"}
    end
  end

  def cast_input(_value, _constraints), do: :error
end
