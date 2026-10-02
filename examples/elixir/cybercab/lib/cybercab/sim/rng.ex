defmodule Cybercab.Sim.Rng do
  @moduledoc """
  A small seeded random source (SplitMix64), so a simulation can be replayed exactly: the
  same generator as the Rust Cybercab's, giving the same numbers from the same seed.

  Like `:rand`, it keeps its state in the calling process: `seed/1` starts a stream, and
  each draw advances it.
  """

  import Bitwise

  @mask 0xFFFF_FFFF_FFFF_FFFF
  @key __MODULE__

  def seed(seed), do: Process.put(@key, seed &&& @mask)

  def next_u64 do
    state = Process.get(@key) || raise "seed the generator first"
    state = state + 0x9E37_79B9_7F4A_7C15 &&& @mask
    Process.put(@key, state)
    z = state
    z = bxor(z, z >>> 30) * 0xBF58_476D_1CE4_E5B9 &&& @mask
    z = bxor(z, z >>> 27) * 0x94D0_49BB_1331_11EB &&& @mask
    bxor(z, z >>> 31)
  end

  @doc "Uniform in `[0, 1)`."
  def unit, do: (next_u64() >>> 11) / 9_007_199_254_740_992

  def between(low, high), do: low + (high - low) * unit()

  def below(n), do: rem(next_u64(), max(n, 1))

  def chance(probability), do: unit() < probability

  def pick(items) when is_tuple(items), do: elem(items, below(tuple_size(items)))
  def pick(items) when is_list(items), do: Enum.at(items, below(length(items)))

  @doc "Picks an index with probability proportional to its weight."
  def weighted(weights) do
    target = unit() * Enum.sum(weights)
    pick_weighted(weights, target, 0, length(weights))
  end

  defp pick_weighted([], _target, _i, count), do: count - 1
  defp pick_weighted([weight | _], target, i, _count) when target < weight, do: i

  defp pick_weighted([weight | rest], target, i, count),
    do: pick_weighted(rest, target - weight, i + 1, count)
end
