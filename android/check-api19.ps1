# Fail if the .so needs a symbol that Android 4.4 (API 19) does not provide.
# Usage: .\check-api19.ps1 <path-to-libplatomon_android.so>   (after . .\env-android.ps1)
#        -Api 23 checks against another API level instead.
param([Parameter(Mandatory)] [string]$So, [int]$Api = 19)
$ErrorActionPreference = 'Stop'
$sys = "$env:ANDROID_NDK_HOME\toolchains\llvm\prebuilt\windows-x86_64\sysroot\usr\lib\arm-linux-androideabi\$Api"
$have = foreach ($lib in 'libc.so', 'libm.so', 'libdl.so', 'liblog.so', 'libandroid.so') {
    llvm-readelf --dyn-syms "$sys\$lib" | Select-String ' DEFAULT ' | ForEach-Object { ($_.Line -split '\s+')[-1] -replace '@.*', '' }
}
$hs = [Collections.Generic.HashSet[string]]::new([string[]]$have)
$strong = llvm-readelf --dyn-syms $So | Select-String ' UND ' | Where-Object { $_.Line -notmatch ' WEAK ' } |
    ForEach-Object { ($_.Line -split '\s+')[-1] -replace '@.*', '' } | Where-Object { $_ }
$missing = @($strong | Where-Object { -not $hs.Contains($_) })
$entry = [bool](llvm-readelf --dyn-syms $So | Select-String ' GLOBAL .* ANativeActivity_onCreate$')
"strong undefined: $(@($strong).Count), missing on API ${Api}: $($missing.Count), entry point exported: $entry"
if ($missing.Count -gt 0) { $missing; throw "library would not load on API $Api" }
if (-not $entry) { throw 'ANativeActivity_onCreate is not exported' }
