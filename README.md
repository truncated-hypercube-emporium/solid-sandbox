# Solid Sandbox

A physics sandbox for Android (4.4+ / 6.0+, armeabi-v7a, NativeActivity, no
Java) and iOS 14+ (iPhone and iPad): a random number of path-traced Platonic and Archimedean solids bounce
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
- `src/app.rs` — the app without its platform (settings on disk, taps,
  frames, drawing), driven by `lib.rs` on Android and `ios.rs` on iOS
- `ios/` — the iOS shell: Objective-C UIKit view (`SolidSandbox/main.m`),
  the C header, an XcodeGen project and the Rust build script
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

## Building for iOS

On a Mac with Xcode 15+, Rust and [XcodeGen](https://github.com/yonaskolb/XcodeGen):

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios
cd ios
xcodegen generate
open SolidSandbox.xcodeproj
```

Pick your team under Signing & Capabilities (or set `DEVELOPMENT_TEAM` in
`ios/project.yml`), then build and run. The "Build Rust core" phase runs
`ios/build-rust.sh`, which builds `libsolid_sandbox.a` for the device or the
simulator into `target/ios-<platform>/`.

iOS has no back key: close a dialog with Cancel, its button, or a tap outside
it. The app runs landscape, inside the safe area, and keeps settings and
palettes in Application Support.

## Signing

APKs are signed with `platomon.keystore` in the tools folder, its password
in `platomon.keystore.pass` next to it. Neither is in this repository; to
make your own:

```powershell
keytool -genkeypair -keystore "$env:LOCALAPPDATA\platomon-android-tools\platomon.keystore" -alias platomon -keyalg RSA -keysize 2048 -validity 10000
```

## Licence

MIT OR Apache-2.0