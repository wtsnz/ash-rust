Mix.install(
  [
    {:ash, "~> 3.0"},
    {:ash_graphql, "~> 1.3"},
    {:absinthe, "~> 1.7"},
    {:benchee, "~> 1.0"}
  ],
  config: [ash: [default_string_length_count: :codepoints]]
)

Code.require_file("embedded_mode.exs", __DIR__)

Logger.configure(level: :warning)

defmodule Support.User do
  use Ash.Resource,
    domain: Support,
    data_layer: Ash.DataLayer.Ets,
    extensions: [AshGraphql.Resource]

  graphql do
    type :user

    queries do
      get :get_user, :read
      list :list_users, :read
    end

    mutations do
      create :create_user, :create
    end
  end

  attributes do
    uuid_primary_key :id
    attribute :name, :string, allow_nil?: false, public?: true
    attribute :email, :string, allow_nil?: false, public?: true
  end

  relationships do
    has_many :tickets, Support.Ticket, destination_attribute: :author_id, public?: true
  end

  actions do
    defaults [:read]

    create :create do
      primary? true
      accept [:name, :email]
    end
  end
end

defmodule Support.Ticket do
  use Ash.Resource,
    domain: Support,
    data_layer: Ash.DataLayer.Ets,
    extensions: [AshGraphql.Resource]

  graphql do
    type :ticket

    queries do
      get :get_ticket, :read
      list :list_tickets, :read
    end

    mutations do
      create :open_ticket, :open
    end
  end

  attributes do
    uuid_primary_key :id
    attribute :title, :string, allow_nil?: false, public?: true
    attribute :status, :atom, constraints: [one_of: [:open, :closed]], default: :open, public?: true
    attribute :priority, :integer, allow_nil?: false, default: 1, public?: true
    attribute :author_id, :uuid, allow_nil?: false, public?: true
  end

  relationships do
    belongs_to :author, Support.User, source_attribute: :author_id, destination_attribute: :id, public?: true
  end

  actions do
    read :read do
      primary? true
      pagination keyset?: true, default_limit: 20
    end

    create :open do
      primary? true
      accept [:title, :status, :priority, :author_id]
    end
  end
end

defmodule Support do
  use Ash.Domain,
    extensions: [AshGraphql.Domain],
    validate_config_inclusion?: false

  resources do
    resource Support.User
    resource Support.Ticket
  end
end

defmodule Support.Schema do
  use Absinthe.Schema
  use AshGraphql, domains: [Support]

  query do
  end

  mutation do
  end
end

defmodule GraphqlBenchRunner do
  def run do
    IO.puts("\n========================================================")
    IO.puts("    Ash Elixir + Absinthe GraphQL Benchmark Suite       ")
    IO.puts("========================================================\n")

    # Seed 5 authors
    author_ids =
      for i <- 1..5 do
        {:ok, user} =
          Ash.create(Support.User, %{
            name: "Staff Engineer ##{i}",
            email: "staff#{i}@company.com"
          })

        user.id
      end

    first_author_id = hd(author_ids)

    # Seed 100 tickets
    first_ticket_id =
      Enum.reduce(1..100, nil, fn i, acc ->
        author_id = Enum.at(author_ids, rem(i - 1, length(author_ids)))
        status = if rem(i, 2) == 0, do: :open, else: :closed
        priority = rem(i, 5) + 1

        {:ok, ticket} =
          Ash.create(Support.Ticket, %{
            title: "Incident ##{i}: Database connection pool exhausted",
            status: status,
            priority: priority,
            author_id: author_id
          })

        if i == 1, do: ticket.id, else: acc
      end)

    single_query = """
    query {
      getTicket(id: "#{first_ticket_id}") {
        id
        title
        status
        priority
      }
    }
    """

    collection_query = """
    query {
      listTickets(first: 100) {
        results {
          id
          title
          status
          priority
        }
      }
    }
    """

    filter_sort_query = """
    query {
      listTickets(
        filter: { status: { eq: "open" } }
        sort: [{ field: PRIORITY, order: DESC }]
        first: 50
      ) {
        results {
          id
          title
          priority
        }
      }
    }
    """

    relay_query = """
    query {
      listTickets(first: 20) {
        count
        startKeyset
        endKeyset
        results {
          id
          title
        }
      }
    }
    """

    nested_query = """
    query {
      listTickets(first: 100) {
        results {
          id
          title
          author {
            id
            name
            email
          }
        }
      }
    }
    """

    mutation_query = """
    mutation {
      openTicket(input: {
        title: "Production Alert: High Error Rate",
        status: "open",
        priority: 1,
        authorId: "#{first_author_id}"
      }) {
        result {
          id
          title
          status
        }
      }
    }
    """

    # Sanity checks
    {:ok, %{data: single_res}} = Absinthe.run(single_query, Support.Schema)
    unless single_res["getTicket"]["id"] == to_string(first_ticket_id), do: raise("single_res failed")

    {:ok, %{data: col_res}} = Absinthe.run(collection_query, Support.Schema)
    unless length(col_res["listTickets"]["results"]) == 100, do: raise("col_res failed")

    {:ok, %{data: nest_res}} = Absinthe.run(nested_query, Support.Schema)
    unless length(nest_res["listTickets"]["results"]) == 100, do: raise("nest_res failed")

    {:ok, _} = Absinthe.run(filter_sort_query, Support.Schema)
    {:ok, _} = Absinthe.run(relay_query, Support.Schema)
    {:ok, _} = Absinthe.run(mutation_query, Support.Schema)

    warmup = 1
    bench_time = 3

    AshBench.EmbeddedMode.enter!()

    Benchee.run(
      %{
        "1. Single Record by ID" => fn ->
          {:ok, _} = Absinthe.run(single_query, Support.Schema)
        end,
        "2. 100 Tickets Collection" => fn ->
          {:ok, _} = Absinthe.run(collection_query, Support.Schema)
        end,
        "3. Filtered & Sorted (50 items)" => fn ->
          {:ok, _} = Absinthe.run(filter_sort_query, Support.Schema)
        end,
        "4. Keyset Pagination (first: 20)" => fn ->
          {:ok, _} = Absinthe.run(relay_query, Support.Schema)
        end,
        "5. Nested Relationship (100 Tickets + Author)" => fn ->
          {:ok, _} = Absinthe.run(nested_query, Support.Schema)
        end,
        "6. Mutation: openTicket (Validation + Action)" => fn ->
          {:ok, _} = Absinthe.run(mutation_query, Support.Schema)
        end
      },
      warmup: warmup,
      time: bench_time,
      memory_time: 1,
      print: [fast_warning: false]
    )

    AshBench.EmbeddedMode.restore!()
  end
end

GraphqlBenchRunner.run()
