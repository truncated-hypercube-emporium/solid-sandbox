# Environment for cross-compiling platomon for Android 4.4 (API 19, armeabi-v7a)
# with NDK r25c. Dot-source it:  . .\env-android.ps1
# Tools are expected in %LOCALAPPDATA%\platomon-android-tools (see build-apk.ps1).
$tools = Join-Path $env:LOCALAPPDATA 'platomon-android-tools'
$ndk = Join-Path $tools 'ndk\android-ndk-r25c'
$llvm = Join-Path $ndk 'toolchains\llvm\prebuilt\windows-x86_64\bin'
$env:ANDROID_NDK_HOME = $ndk
$env:CARGO_TARGET_ARMV7_LINUX_ANDROIDEABI_LINKER = Join-Path $llvm 'armv7a-linux-androideabi19-clang.cmd'
$env:CC_armv7_linux_androideabi = Join-Path $llvm 'armv7a-linux-androideabi19-clang.cmd'
$env:AR_armv7_linux_androideabi = Join-Path $llvm 'llvm-ar.exe'
$env:BUILD_TOOLS = Join-Path $tools 'build-tools\android-13'
$env:ANDROID_JAR = Join-Path $tools 'platform-19\android-4.4.2\android.jar'
$env:JAVA_HOME = (Get-ChildItem (Join-Path $tools 'jdk') -Directory | Select-Object -First 1).FullName
$env:Path = "$env:JAVA_HOME\bin;$env:BUILD_TOOLS;$llvm;$env:Path"
