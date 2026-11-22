# Solid Sandbox

A physics sandbox for Android (4.4+ / 6.0+, armeabi-v7a, NativeActivity, no
Java): a random number of path-traced Platonic and Archimedean solids bounce
around a polyhedral container, drawn as coloured ASCII glyphs. Tap a solid to
kick it.

Features: sphere or exact face/corner collisions, gravity from the
accelerometer, mass variability, elasticity and damping with drag, collision
damage counters, weak and strong (spin-driven shock wave) explosions with
flying faces and sparks, four glow levels with white or inherited glow,
generated 3- to 6-bit palettes, a dynamic camera, and live stats.

The three buttons top right (hidden until you tap the screen) open
**Settings**, **Simulation Parameters** and **Colour**.

## Layout

- `src/` — the sandbox: `physics.rs`, `sandbox.rs` (rendering), `ui.rs`,
  `settings.rs`, `palette.rs`, and `lib.rs` (the NativeActivity glue)
- `android/` — shared Android helpers: bitmap font, API-19 compatibility
  shims, toolchain environment and the API-level symbol check
- `platomon/` — the platomon path-tracing library the scene is rendered with
  (built without its `monitor` feature)
- `examples/terminal.rs` — run the sandbox in a desktop terminal

## Building

Needs Rust with the `armv7-linux-androideabi` target, NDK r25c, Android
build-tools and a JDK in `%LOCALAPPDATA%\platomon-android-tools` (see
`android/env-android.ps1` for the exact paths).

```powershell
powershell -ExecutionPolicy Bypass -File build-apk.ps1 -Api 23   # Android 6.0+ -> dist\solid-sandbox-<version>-android6.apk
powershell -ExecutionPolicy Bypass -File build-apk.ps1           # Android 4.4+ -> dist\solid-sandbox-<version>-android.apk
```

Tests run on the desktop:

```powershell
cargo test --release
```

## Signing

APKs are signed with `platomon.keystore` in the tools folder, its password
in `platomon.keystore.pass` next to it. Neither is in this repository; to
make your own:

```powershell
keytool -genkeypair -keystore "$env:LOCALAPPDATA\platomon-android-tools\platomon.keystore" -alias platomon -keyalg RSA -keysize 2048 -validity 10000
```

## Licence

MIT OR Apache-2.0