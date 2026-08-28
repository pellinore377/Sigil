# Map style

`style-light.json` is the MapLibre style Sigil renders location bubbles and the
map page with. It is written against the **Protomaps Basemap v4** schema.

Sigil does **not** ship a map server, and there is no default tile URL baked
into the app. The engine resolves a style URL in this order
(`engine/src/maps/mod.rs`):

1. `mapStyleUrl` in the local `settings.json` — an explicit override.
2. `m.tile_server.map_style_url` (MSC3488) from **the user's own homeserver's**
   `.well-known/matrix/client`.
3. Nothing. `map.config` reports `configured: false` and the map surfaces
   degrade rather than pointing anywhere.

So a user on some other homeserver gets that homeserver's tile server, or no
map at all. Self-hosters wanting maps should publish their own basemap and
advertise it under `m.tile_server`.

This file is a **source-of-truth artifact for deployment**, not something the
app loads — the app fetches whatever URL the resolution above yields. Editing
it changes nothing until it is published to a tile host.

## Why the water layer is split in two

The Protomaps `water` source-layer is **mixed geometry**: lakes, reservoirs,
basins and the coastline arrive as polygons, but rivers, streams and canals
arrive as **LineStrings** (a single z10 tile can hold 82 of them against 5
polygons).

A MapLibre `fill` layer does not skip line geometry — it hands those vertices
straight to the polygon tessellator, so a meandering creek is filled as a
polygon spanning its own meander. Rendered that way, every creek becomes a lake,
and the map shows large bodies of water where there is none.

So the layer is split, and each half is pinned to the geometry it can actually
draw:

- `water` — `fill`, filtered to `["==", ["geometry-type"], "Polygon"]`
- `water-lines` — `line`, filtered to `LineString`, width interpolated by zoom
  and wider for `kind == "river"` than for streams and canals

Labels are split the same way, because placement differs: a polygon takes a
point label at its centre, while a line needs `symbol-placement: line` or the
name lands at the midpoint of a meander, nowhere near the water it names.

`water` is the only mixed-geometry layer in this schema that feeds a fill —
`earth`, `landcover`, `landuse` and `buildings` are polygon-only, and
`boundaries` and `roads` are lines drawn by line layers. Any new fill layer
added against this source should still be checked by decoding a tile and
looking at what geometry types the layer actually contains.

## Coverage

The style is written against a North America archive (bounds -172,18 to
-52,72). A pin outside the archive's bounds renders as empty canvas, which is a
property of the archive rather than of the style — point it at a
planet-scale source and that goes away.
