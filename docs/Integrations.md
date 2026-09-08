# Maps and providers

Both services are disabled until configured. Admin routes use the installation token; client routes require an active native device credential. Maps and external queries reveal access timing and query contents to the homeserver. Provider requests reveal the disclosed query to the selected provider. Received cards use encrypted snapshots and make no provider requests.

## Local maps

`GET/PUT /admin/v0/maps` accepts `{"expected_revision":0,"settings":{"archive":"/maps/region.pmtiles","assets":"/maps/assets"}}`. Set `settings` to null to disable. Mount operator-selected files read-only into the container; keep them outside Sigil's private database directory. Replace files atomically, then update configuration. Do not modify a live archive in place.

Embedded martin-core serves PMTiles v3 with uncompressed/gzip directories, zooms through 26, tiles up to 4 MiB and metadata/directories up to 1 MiB. No Postgres or remote tile source is used. Supply licensed map data and retain attribution. `assets/style.json` must be MapLibre style version 8; sources use `/client/v0/maps/tiles.json`, and glyph/sprite URLs use `/client/v0/maps/assets/…`. External URLs and style imports are rejected. Attribution becomes escaped text with credit links retained as text. Assets may be JSON, PNG or PBF, at most 4 MiB each.

Authenticated `GET /client/v0/maps` reports availability. Its `/style.json`, `/tiles.json`, `/tiles/{z}/{x}/{y}` and `/assets/{path}` children serve local data. Missing tiles return 204. Native `MapTile` preserves any advertised compression; the renderer adapter must decode that encoding with its own resource limits.

Native location cards support a one-time position, dropped pin or live share for 15 minutes, one hour or eight hours. `location_jobs` returns durable sampling work and pending encrypted stops; `stop_location` persists cancellation before network delivery. Sample updates are at least ten seconds apart. Expiry, group closure/removal and local cancellation stop sampling. GPS permissions, foreground/background scheduling and map rendering belong to platform adapters. Do not claim operating-system delivery from these backend tests.

## Configured providers

`GET/PUT /admin/v0/services` uses `expected_revision`, `per_account_daily`, `total_daily` and up to eight `providers`. Each entry has `provider`, `secret` and an egress `exceptions` array (empty for public HTTPS). A provider has `id`, `kind`, HTTPS `endpoint`, canonical SigilText `attribution`, optional `version` and `source_url`. Secret updates are `{"action":"keep"}`, `{"action":"clear"}` or `{"action":"set","value":"…"}`; readback exposes only `has_secret`. Endpoint changes cannot retain a secret implicitly. Exceptions explicitly bind a host/port to permitted CIDRs and an optional DER CA certificate. Redirects and ambient proxies are disabled.

Kinds: `google_translate` (Basic v2), `libre_translate`, `wiktionary` (REST definition base), `dictionary_index`, `open_meteo` (forecast endpoint) and `geocoder` (Open-Meteo-compatible search). A self-hosted dictionary index accepts JSON `{"word":"…","language":"en"}` and returns canonical `sigil_protocol::text::service::ResultData::Definition`. Deploy that index separately with its dataset attribution/version. No dataset is bundled.

`GET /client/v0/services` returns the public catalog. `POST /client/v0/services/resolve` requires its revision, the entire disclosed provider descriptor, a typed query and optional `refresh`. Native `prepare_service_query` persists disclosure without network access. Call `resolve_service_query` only after acceptance, on a worker; it stores the result before `service_card` creates an encrypted sendable snapshot. Discard drafts after sending or abandonment. Refresh uses a new draft ID. Location search returns choices; weather requires an explicit selected place/timezone.

Requests are bounded to 64 KiB, responses to 256 KiB, four provider workers and three seconds per exchange. Account/global daily budgets survive restart; failed and cached requests consume budget. Dictionary results cache for seven days in bounded server memory; explicit refresh bypasses the cache. Configuration revisions invalidate cached results. Translation, location and weather queries are never repeated automatically. Provider errors permit explicit retry or sending literal text.
