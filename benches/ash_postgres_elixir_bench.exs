Mix.install(
  [
    {:ash, "~> 3.0"},
    {:ash_postgres, "~> 2.0"},
    {:ecto, "~> 3.12"},
    {:ecto_sql, "~> 3.12"},
    {:postgrex, "~> 0.19"},
    {:benchee, "~> 1.0"}
  ],
  config: [ash: [default_string_length_count: :codepoints]]
)

Code.require_file("embedded_mode.exs", __DIR__)

Logger.configure(level: :error)

database_url =
  System.get_env("DATABASE_URL") ||
    "postgres://ash_user:ash_password@localhost:5433/ash_benchmark"

defmodule Bench.Repo do
  use AshPostgres.Repo,
    otp_app: :bench_app,
    warn_on_missing_ash_functions?: false

  def min_pg_version do
    %Version{major: 16, minor: 0, patch: 0}
  end
end

Application.put_env(:bench_app, Bench.Repo, [
  url: database_url,
  pool_size: 20
])

{:ok, _} = Bench.Repo.start_link()

Ecto.Adapters.SQL.query!(Bench.Repo, "DROP TABLE IF EXISTS tickets CASCADE")
Ecto.Adapters.SQL.query!(Bench.Repo, "DROP TABLE IF EXISTS users CASCADE")

Ecto.Adapters.SQL.query!(Bench.Repo, """
CREATE TABLE users (
  id UUID PRIMARY KEY,
  name TEXT NOT NULL,
  email TEXT NOT NULL
)
""")

Ecto.Adapters.SQL.query!(Bench.Repo, """
CREATE TABLE tickets (
  id UUID PRIMARY KEY,
  title TEXT NOT NULL,
  status TEXT NOT NULL DEFAULT 'open',
  priority BIGINT NOT NULL DEFAULT 1,
  user_id UUID REFERENCES users(id)
)
""")

defmodule Bench.User do
  use Ash.Resource,
    domain: Bench.Domain,
    data_layer: AshPostgres.DataLayer

  postgres do
    table "users"
    repo Bench.Repo
  end

  attributes do
    uuid_primary_key :id
    attribute :name, :string, allow_nil?: false, public?: true
    attribute :email, :string, allow_nil?: false, public?: true
  end

  relationships do
    has_many :tickets, Bench.Ticket, destination_attribute: :user_id, public?: true
  end

  aggregates do
    count :ticket_count, :tickets
    count :open_ticket_count, :tickets do
      filter expr(status == "open")
    end
  end

  actions do
    defaults [:read]

    create :create do
      primary? true
      accept [:name, :email]
    end
  end
end

defmodule Bench.Ticket do
  use Ash.Resource,
    domain: Bench.Domain,
    data_layer: AshPostgres.DataLayer

  postgres do
    table "tickets"
    repo Bench.Repo
  end

  attributes do
    uuid_primary_key :id
    attribute :title, :string, allow_nil?: false, public?: true
    attribute :status, :string, default: "open", public?: true
    attribute :priority, :integer, default: 1, public?: true
    attribute :user_id, :uuid, public?: true
  end

  relationships do
    belongs_to :user, Bench.User, source_attribute: :user_id, destination_attribute: :id, public?: true
  end

  actions do
    defaults [:read]

    create :open do
      primary? true
      accept [:title, :status, :priority, :user_id]
    end
  end
end

defmodule Bench.Domain do
  use Ash.Domain, validate_config_inclusion?: false

  resources do
    resource Bench.User
    resource Bench.Ticket
  end
end

defmodule PostgresBenchRunner do
  require Ash.Query

  def run do
    IO.puts("\n========================================================")
    IO.puts("    Ash Elixir + AshPostgres Real-World Benchmark       ")
    IO.puts("========================================================\n")

    # Seed 5 users
    user_ids =
      for i <- 1..5 do
        {:ok, user} =
          Ash.create(Bench.User, %{
            name: "Staff Engineer ##{i}",
            email: "staff#{i}@company.com"
          })

        user.id
      end

    first_user_id = hd(user_ids)
    first_user = Ash.get!(Bench.User, first_user_id)

    # Seed 100 tickets
    first_ticket_id =
      Enum.reduce(1..100, nil, fn i, acc ->
        user_id = Enum.at(user_ids, rem(i - 1, length(user_ids)))
        status = if rem(i, 2) == 0, do: "open", else: "closed"
        priority = rem(i, 5) + 1

        {:ok, ticket} =
          Ash.create(Bench.Ticket, %{
            title: "Incident ##{i}: Database connection pool exhausted",
            status: status,
            priority: priority,
            user_id: user_id
          })

        if i == 1, do: ticket.id, else: acc
      end)

    # Sanity checks
    fetched = Ash.get!(Bench.Ticket, first_ticket_id)
    unless fetched.id == first_ticket_id, do: raise("sanity check failed: fetched ticket")

    user_with_aggs = Ash.load!(first_user, [:ticket_count, :open_ticket_count])
    unless user_with_aggs.ticket_count > 0, do: raise("sanity check failed: aggregates")

    filtered_query =
      Bench.Ticket
      |> Ash.Query.filter(status == "open")
      |> Ash.Query.sort(priority: :desc)
      |> Ash.Query.limit(50)

    filtered_results = Ash.read!(filtered_query)
    unless length(filtered_results) == 50, do: raise("sanity check failed: filtered query")

    _ =
      Ash.bulk_create!(
        [%{title: "Prime bulk", status: "open", priority: 2, user_id: first_user_id}],
        Bench.Ticket,
        :open,
        return_records?: true
      )

    {:ok, _} =
      Ash.transaction([Bench.User, Bench.Ticket], fn ->
        {:ok, u} = Ash.create(Bench.User, %{name: "Prime Tx User", email: "prime-tx@company.com"})
        {:ok, t} = Ash.create(Bench.Ticket, %{title: "Prime Tx Ticket", status: "open", priority: 1, user_id: u.id})
        {u, t}
      end)

    warmup = 1
    bench_time = 3

    AshBench.EmbeddedMode.enter!()

    Benchee.run(
      %{
        "1. Point Write: Ticket.open (RETURNING *)" => fn ->
          {:ok, _} =
            Ash.create(Bench.Ticket, %{
              title: "Benchmarked Critical Failure",
              status: "open",
              priority: 1,
              user_id: first_user_id
            })
        end,
        "2. Point Read: Ticket.get(id) (Primary Key)" => fn ->
          _ = Ash.get!(Bench.Ticket, first_ticket_id)
        end,
        "3. Filtered & Sorted Query (50 items)" => fn ->
          _ = Ash.read!(filtered_query)
        end,
        "4. Correlated Aggregates (Subqueries)" => fn ->
          _ = Ash.load!(first_user, [:ticket_count, :open_ticket_count])
        end,
        "5. Bulk Ingestion: 100 Tickets Batch" => fn ->
          batch_data =
            for i <- 1..100 do
              %{
                title: "Bulk ticket ##{i}",
                status: "open",
                priority: 2,
                user_id: first_user_id
              }
            end

          _ = Ash.bulk_create!(batch_data, Bench.Ticket, :open, return_records?: true)
        end,
        "6. Transactional Workflow: User + Ticket" => fn ->
          {:ok, _} =
            Ash.transaction([Bench.User, Bench.Ticket], fn ->
              {:ok, u} = Ash.create(Bench.User, %{name: "Tx User", email: "tx@company.com"})
              {:ok, t} = Ash.create(Bench.Ticket, %{title: "Tx Ticket", status: "open", priority: 1, user_id: u.id})
              {u, t}
            end)
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

PostgresBenchRunner.run()
