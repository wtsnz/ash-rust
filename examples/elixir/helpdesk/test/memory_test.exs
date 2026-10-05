defmodule Helpdesk.MemoryTest do
  @moduledoc "The desk on ETS."
  use Helpdesk.DeskCase, layer: Helpdesk.Memory
  require Helpdesk.DeskTests
  Helpdesk.DeskTests.define()
end
