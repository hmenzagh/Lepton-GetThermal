# Lepton-GetThermal

Tauri v2 + Rust + React app for FLIR Lepton thermal camera via PureThermal USB (IOKit).

## Build, sign, notarize & release

### 1. Bump version

Update version in **both** files:
- `src-tauri/tauri.conf.json` → `"version"`
- `src-tauri/Cargo.toml` → `version`

### 2. Build

```bash
npm run tauri build
```

Output:
- `src-tauri/target/release/bundle/macos/Lepton-GetThermal.app`
- `src-tauri/target/release/bundle/dmg/Lepton-GetThermal_<version>_aarch64.dmg`

### 3. Sign

```bash
codesign --deep --force --options runtime \
  --sign "Developer ID Application: Hugo Menzaghi (DU7US9538N)" \
  src-tauri/target/release/bundle/macos/Lepton-GetThermal.app
```

### 4. Notarize

```bash
# Zip the signed app
ditto -c -k --keepParent \
  src-tauri/target/release/bundle/macos/Lepton-GetThermal.app \
  /tmp/Lepton-GetThermal.zip

# Submit and wait
xcrun notarytool submit /tmp/Lepton-GetThermal.zip \
  --apple-id "hugo@menzaghi.eu" \
  --password "<app-specific-password>" \
  --team-id "DU7US9538N" \
  --wait

# Staple the ticket
xcrun stapler staple \
  src-tauri/target/release/bundle/macos/Lepton-GetThermal.app
```

The app-specific password is generated at https://appleid.apple.com (Sign-In and Security → App-Specific Passwords).

### 5. Recreate DMG with signed app

```bash
DMG=src-tauri/target/release/bundle/dmg/Lepton-GetThermal_<version>_aarch64.dmg
rm -f "$DMG"
hdiutil create -volname "Lepton-GetThermal" \
  -srcfolder src-tauri/target/release/bundle/macos/Lepton-GetThermal.app \
  -ov -format UDZO "$DMG"
```

### 6. GitHub release

```bash
gh release create v<version> "$DMG" \
  --title "Lepton-GetThermal v<version>" \
  --notes "Release notes here"
```

## Development

```bash
npm run tauri dev
```

Note: the app icon only appears in production builds, not in dev mode (macOS Tauri limitation).

## Code signing identity

- Certificate: `Developer ID Application: Hugo Menzaghi (DU7US9538N)`
- Apple ID: `hugo@menzaghi.eu`
- Team ID: `DU7US9538N`
