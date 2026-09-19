# Production Elixir releases boot the code server in `:embedded` mode.
# `elixir script.exs` / Mix.install stay in `:interactive`, so every
# `Code.ensure_loaded/1` miss walks the code path. Ash does a lot of
# those optional-module lookups; that tax is why a 100-row read measures
# ~400µs here and ~300µs in a release.
#
# OTP 27+ stores the mode in persistent_term, which `code:ensure_loaded/1`
# reads on the client side. After the workload has been primed we flip
# that flag so the suite matches production.

defmodule AshBench.EmbeddedMode do
  @code_server_key :code_server

  def enter! do
    case :code.get_mode() do
      :embedded ->
        IO.puts("Code server already in :embedded mode")
        :ok

      :interactive ->
        preload_available_modules()
        :persistent_term.put(@code_server_key, :embedded)

        case :code.get_mode() do
          :embedded ->
            IO.puts("Switched code server to :embedded mode (production release behavior)")
            :ok

          other ->
            raise "failed to enter :embedded mode, still #{inspect(other)}"
        end
    end
  end

  def restore! do
    if :persistent_term.get(@code_server_key, :interactive) == :embedded do
      :persistent_term.put(@code_server_key, :interactive)
    end

    :ok
  end

  defp preload_available_modules do
    for {mod, _file, loaded} <- :code.all_available(), loaded != true do
      mod = if is_list(mod), do: List.to_atom(mod), else: mod
      _ = :code.ensure_loaded(mod)
    end

    :ok
  end
end
