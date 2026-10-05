defmodule Helpdesk.SqliteTest do
  @moduledoc "The desk on SQLite."
  use Helpdesk.DeskCase, layer: Helpdesk.Sqlite
  require Helpdesk.DeskTests
  Helpdesk.DeskTests.define()
end
