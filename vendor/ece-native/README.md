Pinned local patch of ece-native 0.5.0, upstream commit
`6b0a3f10377af3c9401a7c1a60b1e34d41155c5d`, `ece/src`.
Original lib.rs SHA256:
`38a057da25992c4ab87e9e9c77899e57e2e763514cf3c993325485abb253fe55`.
The upstream MIT licensing option is selected; its notice is included.

Reproduced before patching: zero `rs` panics, header-only input returns an
unauthenticated empty result, and authenticated padding delimiters other than
1/2 are accepted. RFC8188 requires `rs >= 18`, at least one authenticated record,
and exact final/nonfinal delimiters. This patch adds those checks and regression
tests; upstream cipher/KDF code is unchanged. Dependencies are pinned to the
workspace's resolved versions. Upstream test Lazy values use statics for the
workspace's current Clippy checks. Run `cargo test -p ece-native` from the repo root.

Web-push-native's separate equal-record-size sender issue is corrected by the
Sigil provider framing adapter, which sets the single-record `rs` header to 4096.
This patch is local and has not been submitted upstream or independently audited.
