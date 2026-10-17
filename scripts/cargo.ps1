param()
$CargoArgs=$args
$ErrorActionPreference='Stop'
$root=Split-Path $PSScriptRoot -Parent
if (-not (Test-Path -LiteralPath (Join-Path $root '.tools/cargo/bin/cargo.exe'))) {
    # Public source checkouts use an installed GNU Rust/MinGW toolchain.
    $cargoCommand=Get-Command cargo -ErrorAction Stop
    Push-Location $root
    try { & $cargoCommand.Source @CargoArgs; if($LASTEXITCODE -ne 0){exit $LASTEXITCODE} } finally {Pop-Location}
    return
}
$env:CARGO_HOME=Join-Path $root '.tools/cargo'
$env:RUSTUP_HOME=Join-Path $root '.tools/rustup'
$env:GIT_CONFIG_PARAMETERS="'http.proxy=' 'https.proxy='"
$env:CC=Join-Path $root '.tools/llvm-mingw-20261006-msvcrt-x86_64/bin/clang.exe'
$env:AR=Join-Path $root '.tools/llvm-mingw-20261006-msvcrt-x86_64/bin/llvm-ar.exe'
$env:CXX=Join-Path $root '.tools/llvm-mingw-20261006-msvcrt-x86_64/bin/clang++.exe'
# Compute linker paths from this workspace; moving the folder needs no config edits.
$env:CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=$env:CC
$llvmLib=(Join-Path $root '.tools/llvm-mingw-20261006-msvcrt-x86_64/x86_64-w64-mingw32/lib').Replace('\','/')
$rustLib=(Join-Path $env:RUSTUP_HOME 'toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/lib/self-contained').Replace('\','/')
$env:CARGO_ENCODED_RUSTFLAGS=@('-C',"link-arg=-L$llvmLib",'-C',"link-arg=-L$rustLib") -join [char]31
$env:Path=(Join-Path $root '.tools/llvm-mingw-20261006-msvcrt-x86_64/bin')+';'+(Join-Path $env:RUSTUP_HOME 'toolchains/stable-x86_64-pc-windows-gnu/lib/rustlib/x86_64-pc-windows-gnu/bin/self-contained')+';'+(Join-Path $env:CARGO_HOME 'bin')+';'+$env:Path
Push-Location $root
try { & (Join-Path $env:CARGO_HOME 'bin/cargo.exe') @CargoArgs; if($LASTEXITCODE -ne 0){exit $LASTEXITCODE} } finally {Pop-Location}

