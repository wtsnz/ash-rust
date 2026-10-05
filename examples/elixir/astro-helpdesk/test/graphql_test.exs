defmodule AstroHelpdesk.GraphqlTest do
  @moduledoc """
  Ports `examples/astro-helpdesk/tests/integration_test.rs`: the same GraphQL documents, sent to
  the endpoint as the Astro frontend sends them.
  """
  use ExUnit.Case, async: false

  import Plug.Conn

  alias AstroHelpdesk.Desk.{Representative, Ticket}

  @endpoint AstroHelpdeskWeb.Endpoint

  setup do
    for resource <- [Ticket, Representative], do: Ash.DataLayer.Ets.stop(resource)
    AstroHelpdesk.Seed.run()
    :ok
  end

  defp post(query, variables \\ %{}) do
    conn =
      Plug.Test.conn(:post, "/graphql", Jason.encode!(%{query: query, variables: variables}))
      |> put_req_header("content-type", "application/json")
      |> @endpoint.call(@endpoint.init([]))

    assert conn.status == 200
    Jason.decode!(conn.resp_body)
  end

  test "health answers OK" do
    conn = Plug.Test.conn(:get, "/health") |> @endpoint.call(@endpoint.init([]))
    assert {conn.status, conn.resp_body} == {200, "OK"}
  end

  test "reads the seeded tickets with their authors" do
    body =
      post("""
      query {
        listTickets {
          results { id title status priority author { id name role } }
        }
      }
      """)

    refute body["errors"], inspect(body)
    tickets = body["data"]["listTickets"]["results"]
    assert length(tickets) == 3
    assert Enum.all?(tickets, &is_binary(&1["author"]["name"]))
    assert Enum.map(tickets, & &1["status"]) |> Enum.sort() == ["IN_PROGRESS", "OPEN", "RESOLVED"]
  end

  test "opens, updates and closes a ticket" do
    title = "Critical test alert: #{Ash.UUID.generate()}"

    body =
      post(
        """
        mutation Open($input: OpenTicketInput!) {
          openTicket(input: $input) {
            result { id title status priority }
            errors { fields message }
          }
        }
        """,
        %{
          input: %{
            title: title,
            description: "Integration test created ticket",
            priority: 1,
            status: "OPEN"
          }
        }
      )

    refute body["errors"], inspect(body)
    payload = body["data"]["openTicket"]
    assert payload["errors"] == []
    assert payload["result"]["title"] == title
    id = payload["result"]["id"]

    body =
      post(
        """
        mutation ChangeStatus($id: ID!, $input: ChangeStatusTicketInput) {
          changeStatusTicket(id: $id, input: $input) {
            result { id status }
            errors { message }
          }
        }
        """,
        %{id: id, input: %{status: "RESOLVED"}}
      )

    assert body["data"]["changeStatusTicket"]["result"]["status"] == "RESOLVED"

    body =
      post(
        """
        mutation Close($id: ID!) {
          closeTicket(id: $id) { result { id } errors { message } }
        }
        """,
        %{id: id}
      )

    assert body["data"]["closeTicket"]["errors"] == []
    assert body["data"]["closeTicket"]["result"]["id"] == id
  end

  test "refuses a ticket whose title is too short or priority out of range" do
    body =
      post(
        """
        mutation Open($input: OpenTicketInput!) {
          openTicket(input: $input) { result { id } errors { fields message } }
        }
        """,
        %{input: %{title: "hey", priority: 9, status: "OPEN"}}
      )

    payload = body["data"]["openTicket"]
    assert payload["result"] == nil

    assert payload["errors"] |> Enum.flat_map(& &1["fields"]) |> Enum.sort() == [
             "priority",
             "title"
           ]
  end

  test "an attachment is base64 text" do
    bytes = <<0, 1, 2, 255>>

    body =
      post(
        """
        mutation Open($input: OpenTicketInput!) {
          openTicket(input: $input) { result { attachment } errors { message } }
        }
        """,
        %{
          input: %{
            title: "With an attachment",
            priority: 3,
            status: "OPEN",
            attachment: Base.encode64(bytes)
          }
        }
      )

    assert body["data"]["openTicket"]["result"]["attachment"] == Base.encode64(bytes)

    body =
      post(
        """
        mutation Open($input: OpenTicketInput!) {
          openTicket(input: $input) { result { id } errors { fields } }
        }
        """,
        %{
          input: %{
            title: "With a bad one",
            priority: 3,
            status: "OPEN",
            attachment: "not base64!"
          }
        }
      )

    assert [%{"fields" => ["attachment"]}] = body["data"]["openTicket"]["errors"]
  end

  test "the Ash resources do what the GraphQL does" do
    rep =
      Ash.create!(Representative, %{
        name: "Alex Mercer",
        email: "alex@support.ash",
        role: "Escalations Lead"
      })

    ticket =
      Ash.create!(
        Ticket,
        %{
          title: "Flaky websocket disconnects in EU cluster",
          description: "Heartbeat latency spikes over 5000ms.",
          status: :open,
          priority: 2,
          author_id: rep.id
        },
        action: :open
      )

    assert ticket.author_id == rep.id

    require Ash.Query

    assert [%{id: id}] =
             Ticket |> Ash.Query.filter(priority <= 2 and author_id == ^rep.id) |> Ash.read!()

    assert id == ticket.id

    updated =
      ticket |> Ash.Changeset.for_update(:change_status, %{status: :in_progress}) |> Ash.update!()

    assert updated.status == :in_progress

    Ash.destroy!(updated, action: :close)
    assert Ash.read!(Ticket |> Ash.Query.filter(author_id == ^rep.id)) == []
  end
end
