defmodule Helpdesk.Sqlite do
  @moduledoc "The helpdesk kept in SQLite, as the Rust desk runs on `ash-sqlite`."
  use Ash.Domain

  resources do
    resource Helpdesk.Sqlite.Ticket do
      define :open_ticket, action: :open, args: [:subject]
      define :close_ticket, action: :close
      define :assign_ticket, action: :assign, args: [:representative_id]
      define :get_ticket, action: :read, get_by: :id
      define :list_tickets, action: :read
      define :analyze_subject, args: [:text]
      define :intake_ticket, action: :intake, args: [:subject]
    end

    resource Helpdesk.Sqlite.Representative do
      define :create_representative, action: :create, args: [:name]
      define :list_representatives, action: :read
      define :get_representative, action: :read, get_by: :id
    end
  end
end
