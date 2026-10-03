# Cybercab city data

Austin as both Cybercab apps see it, so the Rust (`examples/cybercab`) and Elixir
(`examples/elixir/cybercab`) simulations drive the same city:

- `austin.json`: the service zones, the places riders go, and the Supercharger hubs.
- `routes.json`: driving routes between every pair of stops, as encoded polylines, baked
  from OpenStreetMap by `examples/cybercab/tools/bake-routes.mjs`.
