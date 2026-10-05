defmodule Helpdesk.DeskTests do
  @moduledoc """
  What the desk does on any data layer, written once: `Helpdesk.DeskTests.define()` inside a
  module that uses `Helpdesk.DeskCase`.
  """

  defmacro define do
    quote do
      describe "tickets" do
        test "a customer opens a ticket and reads it" do
          customer = customer()
          ticket = @desk.open_ticket!("Broken mouse", actor: customer)

          assert ticket.status == :open
          assert ticket.opener_id == customer.id
          assert ticket.representative_id == nil

          assert [listed] = @desk.list_tickets!(actor: customer)
          assert listed.id == ticket.id
        end

        test "another customer does not see it" do
          @desk.open_ticket!("Broken mouse", actor: customer())
          assert @desk.list_tickets!(actor: customer()) == []
        end

        test "a representative sees the unassigned queue, then only what is assigned to them" do
          alice = @desk.create_representative!("Alice")
          bob = @desk.create_representative!("Bob")
          as_alice = representative(alice)
          as_bob = representative(bob)

          ticket = @desk.open_ticket!("Printer", actor: customer())
          assert [_] = @desk.list_tickets!(actor: as_bob)

          @desk.assign_ticket!(ticket, alice.id, actor: as_alice)

          assert @desk.list_tickets!(actor: as_bob) == []
          assert [seen] = @desk.list_tickets!(actor: as_alice)
          assert seen.representative_id == alice.id
        end

        test "the opener still sees a ticket once it's assigned" do
          alice = @desk.create_representative!("Alice")
          customer = customer()
          ticket = @desk.open_ticket!("Printer", actor: customer)
          @desk.assign_ticket!(ticket, alice.id, actor: representative(alice))

          assert [_] = @desk.list_tickets!(actor: customer)
        end

        test "the opener and the assignee can close it" do
          alice = @desk.create_representative!("Alice")
          customer = customer()
          as_alice = representative(alice)

          first = @desk.open_ticket!("One", actor: customer)
          assert @desk.close_ticket!(first, actor: customer).status == :closed

          second = @desk.open_ticket!("Two", actor: customer)
          assigned = @desk.assign_ticket!(second, alice.id, actor: as_alice)
          assert @desk.close_ticket!(assigned, actor: as_alice).status == :closed
        end

        test "another representative can't close an assigned ticket" do
          alice = @desk.create_representative!("Alice")
          bob = @desk.create_representative!("Bob")
          ticket = @desk.open_ticket!("Printer", actor: customer())
          assigned = @desk.assign_ticket!(ticket, alice.id, actor: representative(alice))

          # By id, Bob's read policy hides it: not found.
          assert {:error, %Ash.Error.Invalid{errors: [%Ash.Error.Query.NotFound{}]}} =
                   @desk.get_ticket(assigned.id, actor: representative(bob))

          # With the record in hand, his write policy refuses: forbidden.
          assert {:error, %Ash.Error.Forbidden{}} =
                   @desk.close_ticket(assigned, actor: representative(bob))
        end

        test "a customer can't assign" do
          alice = @desk.create_representative!("Alice")
          customer = customer()
          ticket = @desk.open_ticket!("Printer", actor: customer)

          assert {:error, %Ash.Error.Forbidden{}} =
                   @desk.assign_ticket(ticket, alice.id, actor: customer)
        end

        test "opening without an actor is forbidden" do
          assert {:error, %Ash.Error.Forbidden{}} = @desk.open_ticket("Printer")
        end

        test "a read without an actor is forbidden" do
          @desk.open_ticket!("Printer", actor: customer())
          assert {:error, %Ash.Error.Forbidden{}} = @desk.list_tickets()
        end

        test "a caller's filter and the policy's filter combine" do
          customer = customer()
          @desk.open_ticket!("open one", actor: customer)
          closed = @desk.open_ticket!("close me", actor: customer)
          @desk.close_ticket!(closed, actor: customer)

          open_only =
            @ticket
            |> Ash.Query.filter_input(%{
              and: [%{status: %{eq: :open}}, %{status: %{not_eq: :closed}}]
            })
            |> Ash.read!(actor: customer)

          assert [%{subject: "open one"}] = open_only
        end

        test "an input the action doesn't accept is rejected" do
          assert {:error,
                  %Ash.Error.Invalid{errors: [%Ash.Error.Invalid.NoSuchInput{input: :status}]}} =
                   @ticket
                   |> Ash.Changeset.for_create(:open, %{subject: "hi", status: :closed},
                     actor: customer()
                   )
                   |> Ash.create()
        end

        test "sorts by subject" do
          customer = customer()
          @desk.open_ticket!("zeta", actor: customer)
          @desk.open_ticket!("alpha", actor: customer)

          sorted = @ticket |> Ash.Query.sort(:subject) |> Ash.read!(actor: customer)
          assert Enum.map(sorted, & &1.subject) == ["alpha", "zeta"]
        end

        test "its changeset holds the changes before it commits" do
          customer = customer()

          changeset =
            Ash.Changeset.for_create(@ticket, :open, %{subject: "Inspect me"}, actor: customer)

          assert Ash.Changeset.get_attribute(changeset, :status) == :open
          assert Ash.Changeset.get_attribute(changeset, :opener_id) == customer.id
          assert Ash.create!(changeset).subject == "Inspect me"
        end

        test "it keeps the optional fields it is given" do
          customer = customer()

          ticket =
            @ticket
            |> Ash.Changeset.for_create(
              :open,
              %{
                subject: "Everything",
                estimate: 2.5,
                due_on: ~D[2026-10-31],
                attachment: <<0, 1, 2, 255>>,
                requester_email: "Ada@Example.com"
              },
              actor: customer
            )
            |> Ash.create!()

          found = @desk.get_ticket!(ticket.id, actor: customer)
          assert found.estimate == 2.5
          assert found.due_on == ~D[2026-10-31]
          assert found.attachment == <<0, 1, 2, 255>>
          assert to_string(found.requester_email) == "Ada@Example.com"
        end
      end

      describe "validations" do
        test "a subject is present and at least two characters" do
          customer = customer()

          assert {:error, %Ash.Error.Invalid{errors: errors}} =
                   @desk.open_ticket("x", actor: customer)

          assert [%Ash.Error.Changes.InvalidAttribute{field: :subject}] = errors

          assert {:error,
                  %Ash.Error.Invalid{errors: [%Ash.Error.Changes.Required{field: :subject}]}} =
                   @ticket
                   |> Ash.Changeset.for_create(:open, %{}, actor: customer)
                   |> Ash.create()
        end

        test "a representative's name is present and at least two characters" do
          assert {:error, %Ash.Error.Invalid{}} = @desk.create_representative("A")
          assert {:error, %Ash.Error.Invalid{}} = @desk.create_representative(nil)
          assert {:ok, _} = @desk.create_representative("Al")
        end
      end

      describe "representatives" do
        test "can be listed" do
          @desk.create_representative!("Alice")
          @desk.create_representative!("Bob")
          assert length(@desk.list_representatives!()) == 2
        end

        test "are paged by keyset and by offset, with a count" do
          for name <- ["Ann", "Bea", "Cyd"], do: @desk.create_representative!(name)

          page = Ash.read!(@representative, page: [limit: 2, count: true])
          assert length(page.results) == 2
          assert page.count == 3
          assert page.more?

          offset = Ash.read!(@representative, page: [limit: 2, offset: 2])
          assert length(offset.results) == 1

          keyset = Ash.read!(@representative, page: [limit: 2], action: :read)
          [first | _] = keyset.results
          assert %Ash.Page.Offset{} = keyset
          assert first.__metadata__ != nil

          sorted = @representative |> Ash.Query.sort(:name) |> Ash.read!(page: [limit: 1])
          assert [%{name: "Ann"}] = sorted.results
        end
      end

      describe "generic actions" do
        test "analyze_subject counts words and spots urgency" do
          assert %{word_count: 3, urgent: true} =
                   @desk.analyze_subject!("Printer is jammed!", actor: customer())
        end

        test "analyze_subject requires an actor" do
          assert {:error, %Ash.Error.Forbidden{}} = @desk.analyze_subject("hi")
        end

        test "manual intake skips the data layer" do
          customer = customer()
          ticket = @desk.intake_ticket!("From email", actor: customer)

          assert ticket.status == :open
          assert @desk.list_tickets!(actor: customer) == []

          stored = Helpdesk.IntakeStore.get(ticket.id)
          assert stored.subject == "From email"
          assert stored.opener_id == customer.id
        end
      end

      describe "calculations" do
        test "a calculation is not loaded until asked" do
          customer = customer()
          ticket = @desk.open_ticket!("hello world", actor: customer)
          assert %Ash.NotLoaded{} = ticket.subject_length

          [listed] = @desk.list_tickets!(actor: customer)
          assert %Ash.NotLoaded{} = listed.subject_length

          loaded = @desk.get_ticket!(ticket.id, actor: customer, load: [:subject_length])
          assert loaded.subject_length == 11
        end

        test "a calculation filters without being loaded" do
          customer = customer()
          @desk.open_ticket!("hi", actor: customer)
          @desk.open_ticket!("hello world", actor: customer)

          [long] =
            @ticket |> longer_than(5) |> Ash.read!(actor: customer)

          # Whether the calculation then comes back loaded is the data layer's business: ETS
          # evaluates it onto the records it filters, SQLite filters in SQL and doesn't.
          assert long.subject == "hello world"
        end
      end

      describe "relationships" do
        test "a ticket loads its representative only when asked" do
          alice = @desk.create_representative!("Alice")
          customer = customer()
          ticket = @desk.open_ticket!("Printer", actor: customer)
          @desk.assign_ticket!(ticket, alice.id, actor: representative(alice))

          [plain] = @desk.list_tickets!(actor: customer)
          assert %Ash.NotLoaded{} = plain.representative

          [loaded] = @desk.list_tickets!(actor: customer, load: [:representative])
          assert loaded.representative.name == "Alice"
        end

        test "a ticket with no representative loads none" do
          customer = customer()
          @desk.open_ticket!("Printer", actor: customer)

          [loaded] = @desk.list_tickets!(actor: customer, load: [:representative])
          assert loaded.representative == nil
        end

        test "a representative loads the tickets assigned to them" do
          alice = @desk.create_representative!("Alice")
          as_alice = representative(alice)
          ticket = @desk.open_ticket!("Printer", actor: customer())
          @desk.assign_ticket!(ticket, alice.id, actor: as_alice)

          plain = @desk.get_representative!(alice.id, actor: as_alice)
          assert %Ash.NotLoaded{} = plain.tickets

          loaded = @desk.get_representative!(alice.id, actor: as_alice, load: [:tickets])
          assert [%{id: id}] = loaded.tickets
          assert id == ticket.id
        end

        test "a representative's tickets are the ones the reader may read" do
          alice = @desk.create_representative!("Alice")
          ticket = @desk.open_ticket!("Printer", actor: customer())
          @desk.assign_ticket!(ticket, alice.id, actor: representative(alice))

          loaded = @desk.get_representative!(alice.id, actor: customer(), load: [:tickets])
          assert loaded.tickets == []
        end
      end
    end
  end
end
