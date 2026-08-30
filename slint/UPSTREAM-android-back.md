# Upstream repro: Android back never reaches the UI

For filing against `slint-ui/slint` (component `i-slint-backend-android-activity`).
Observed on Slint 1.17.1, backend-android-activity-06, NativeActivity,
OnePlus 6 / Android 11.

## Symptom

The hardware/gesture back action closes the activity outright. Neither of
the two channels a Slint app could use ever fires:

1. `Window::on_close_requested` — never invoked. (Registered on the app
   window before `run()`; a log line in the handler never prints.)
2. Key events — a `FocusScope` with focus (confirmed: `init => self.focus()`
   plus a `has-focus` probe binding that logs true) receives no key-pressed
   for the back action. `Key.Escape`, the usual mapping, never arrives; no
   other key event arrives at back-press time either.

So an Android Slint app cannot intercept back for in-app navigation — the
platform convention on every other Android toolkit.

## Repro sketch

```slint
export component W inherits Window {
    fs := FocusScope {
        init => { self.focus(); }
        key-pressed(e) => {
            debug("key:", e.text);   // never fires for back
            accept
        }
    }
}
```

```rust
// android_main:
let w = W::new().unwrap();
w.window().on_close_requested(|| {
    eprintln!("close requested");    // never prints; the activity just finishes
    slint::CloseRequestReason::KeepWindowShown.into()
});
w.run().unwrap();
```

Press back: the activity finishes, neither callback logs.

## Expected

Either deliver back as a key event (as winit does for Escape) or fire
`close_requested` so the app can decide, matching
`onBackPressedDispatcher` semantics.

## Notes

- `android-activity`'s `MainEvent::Destroy`/`WindowDestroyed` arrive after
  the fact; too late to veto.
- Kotlin `OnBackPressedCallback` interception isn't reachable from a pure
  NativeActivity + cargo-apk build (no Java compilation step).
