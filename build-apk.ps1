# Build the Solid Sandbox APK (armeabi-v7a).
#
#   powershell -ExecutionPolicy Bypass -File build-apk.ps1           # Android 4.4+ (API 19)
#     -> dist\solid-sandbox-<version>-android.apk
#   powershell -ExecutionPolicy Bypass -File build-apk.ps1 -Api 23   # Android 6.0+ (API 23)
#     -> dist\solid-sandbox-<version>-android6.apk
#
# Tools and signing key live in %LOCALAPPDATA%\platomon-android-tools (see
# android\env-android.ps1 and README.md).
param([ValidateSet(19, 23)] [int]$Api = 19)

$ErrorActionPreference = 'Stop'
$here = $PSScriptRoot
. (Join-Path $here 'android\env-android.ps1')
$suffix = @{ 19 = ''; 23 = '6' }[$Api]
if ($Api -ne 19) {
    # link against the API's NDK sysroot; compile resources against a newer platform
    $llvm = Join-Path $env:ANDROID_NDK_HOME 'toolchains\llvm\prebuilt\windows-x86_64\bin'
    $env:CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER = Join-Path $llvm "armv7a-linux-androideabi$Api-clang.cmd"
    $env:CC_armv7_linux_androideabi = $env:CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER
    $env:ANDROID_JAR = Join-Path $env:LOCALAPPDATA 'platomon-android-tools\sdk\platforms\android-30\android.jar'
    if (-not (Test-Path $env:ANDROID_JAR)) { throw "missing $env:ANDROID_JAR (sdkmanager `"platforms;android-30`")" }
}
Push-Location $here
try {
    $base = if ($env:CARGO_TARGET_DIR) { $env:CARGO_TARGET_DIR } else { Join-Path $here 'target' }
    # separate target dir per API so a change of linker always relinks
    $target = if ($Api -eq 19) { $base } else { "$base-api$Api" }
    $version = (Select-String -Path (Join-Path $here 'Cargo.toml') -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value

    # 1. native library
    cargo build --release --target armv7-linux-androideabi --target-dir $target
    if ($LASTEXITCODE -ne 0) { throw 'cargo build failed' }
    $so = Join-Path $target 'armv7-linux-androideabi\release\libsolid_sandbox.so'
    & (Join-Path $here 'android\check-api19.ps1') $so -Api $Api

    # manifest for this API level
    $mdir = Join-Path $here "build\api$Api"   # aapt insists on the name AndroidManifest.xml
    New-Item -ItemType Directory -Force $mdir | Out-Null
    $manifest = Join-Path $mdir 'AndroidManifest.xml'
    $xml = (Get-Content -Raw (Join-Path $here 'AndroidManifest.xml')) `
        -replace 'android:minSdkVersion="\d+"', "android:minSdkVersion=`"$Api`"" `
        -replace 'android:targetSdkVersion="\d+"', "android:targetSdkVersion=`"$Api`""
    [IO.File]::WriteAllText($manifest, $xml)

    # 2. launcher icon (generated): three solids-as-discs, each in its own colour
    $res = Join-Path $here 'build\res\drawable'
    New-Item -ItemType Directory -Force $res | Out-Null
    Add-Type -AssemblyName System.Drawing
    $bmp = New-Object System.Drawing.Bitmap 96, 96
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = 'AntiAlias'
    $g.Clear([System.Drawing.Color]::Black)
    $g.DrawRectangle((New-Object System.Drawing.Pen ([System.Drawing.Color]::FromArgb(86, 102, 128)), 3), 6, 6, 84, 84)
    foreach ($d in @(@(18, 52, 26, 0xE0, 0x3A, 0x5A), @(52, 20, 24, 0x3A, 0xB0, 0xF0), @(54, 56, 22, 0xF0, 0xC0, 0x30))) {
        $b = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb($d[3], $d[4], $d[5]))
        $g.FillEllipse($b, $d[0], $d[1], $d[2], $d[2])
    }
    $bmp.Save((Join-Path $res 'icon.png'), [System.Drawing.Imaging.ImageFormat]::Png)
    $g.Dispose(); $bmp.Dispose()

    # 3. package: manifest + resources, then the library
    $stage = Join-Path $here 'build\apk'
    if (Test-Path $stage) { Remove-Item -Recurse -Force $stage }
    New-Item -ItemType Directory -Force (Join-Path $stage 'lib\armeabi-v7a') | Out-Null
    Copy-Item $so (Join-Path $stage 'lib\armeabi-v7a\libsolid_sandbox.so')
    $unaligned = Join-Path $here 'build\unaligned.apk'
    $aligned = Join-Path $here 'build\aligned.apk'
    aapt package -f -M $manifest -S (Join-Path $here 'build\res') -I $env:ANDROID_JAR -F $unaligned
    if ($LASTEXITCODE -ne 0) { throw 'aapt package failed' }
    Push-Location $stage
    try { aapt add $unaligned 'lib/armeabi-v7a/libsolid_sandbox.so' | Out-Null } finally { Pop-Location }
    if ($LASTEXITCODE -ne 0) { throw 'aapt add failed' }
    zipalign -f -p 4 $unaligned $aligned
    if ($LASTEXITCODE -ne 0) { throw 'zipalign failed' }

    # 4. sign (v1 for Android 4.4 and 6, v2 for 7+) with platomon's key.
    $ErrorActionPreference = 'Continue'
    $tools = Join-Path $env:LOCALAPPDATA 'platomon-android-tools'
    $ks = Join-Path $tools 'platomon.keystore'
    $passFile = "$ks.pass"
    if (-not (Test-Path $ks)) { throw "signing key missing: $ks (see README.md: Signing)" }
    New-Item -ItemType Directory -Force (Join-Path $here 'dist') | Out-Null
    $out = (Resolve-Path (Join-Path $here 'dist')).Path + "\solid-sandbox-$version-android$suffix.apk"
    & (Join-Path $env:BUILD_TOOLS 'apksigner.bat') sign --ks $ks --ks-pass "file:$passFile" --ks-key-alias platomon `
        --min-sdk-version $Api --v1-signing-enabled true --v2-signing-enabled true --out $out $aligned
    if ($LASTEXITCODE -ne 0) { throw 'apksigner failed' }
    Write-Host "built $out ($([math]::Round((Get-Item $out).Length / 1MB, 2)) MB)"
}
finally {
    Pop-Location
}
