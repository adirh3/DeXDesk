param(
    [switch]$Test,
    [string]$OutputDirectory = (Join-Path $PSScriptRoot 'dist'),
    [ValidateSet('x64','arm64')]
    [string]$Architecture = 'x64'
)
$ErrorActionPreference = 'Stop'
$previous = @{
    CARGO_HOME=$env:CARGO_HOME; RUSTUP_HOME=$env:RUSTUP_HOME; PATH=$env:PATH
    CARGO_ENCODED_RUSTFLAGS=$env:CARGO_ENCODED_RUSTFLAGS
}
Push-Location $PSScriptRoot
try {
    $portable = Join-Path $PSScriptRoot '.tools\cargo\bin\cargo.exe'
    if (Test-Path -LiteralPath $portable) {
        $env:CARGO_HOME = Join-Path $PSScriptRoot '.tools\cargo'
        $env:RUSTUP_HOME = Join-Path $PSScriptRoot '.tools\rustup'
        $env:PATH = (Join-Path $env:CARGO_HOME 'bin') + ';' + $env:PATH
        $cargo=$portable
    } else {
        $command = Get-Command cargo -ErrorAction Stop
        $cargo = $command.Source
    }
    $linker = $null
    if ($Architecture -eq 'arm64') {
        # Native Windows on ARM build. The gnullvm toolchain links with clang/lld
        # from llvm-mingw, taken from .tools\llvm-mingw-*-aarch64 or PATH.
        $toolchain='1.99.1-aarch64-pc-windows-gnullvm'
        $linker = Get-ChildItem -LiteralPath (Join-Path $PSScriptRoot '.tools') -Directory `
            -Filter 'llvm-mingw-*-aarch64' -ErrorAction SilentlyContinue |
            Sort-Object Name | Select-Object -Last 1 -ExpandProperty FullName
        if ($linker) {
            $env:PATH = (Join-Path $linker 'bin') + ';' + $env:PATH
        } elseif (-not (Get-Command aarch64-w64-mingw32-clang -ErrorAction SilentlyContinue)) {
            throw 'ARM64 builds need llvm-mingw (aarch64) in .tools or on PATH.'
        }
    } else {
        $toolchain='1.99.1-x86_64-pc-windows-gnu'
    }
    $flags = @(if ($env:CARGO_ENCODED_RUSTFLAGS) {
        @($env:CARGO_ENCODED_RUSTFLAGS -split [char]31)
    } elseif ($env:RUSTFLAGS) {
        @($env:RUSTFLAGS -split '\s+' | Where-Object { $_ })
    })
    if ($Architecture -eq 'arm64') {
        # Link libunwind statically so the EXE needs no extra runtime DLLs.
        $flags += '-Ctarget-feature=+crt-static'
    }
    # Remap dependency panic/source locations too, not just stripped debug symbols.
    $remaps = @(
        @($env:USERPROFILE,'user-home'),
        @($env:CARGO_HOME,'cargo-home'),
        @($PSScriptRoot,'DeXDesk'),
        @($linker,'llvm-mingw')
    )
    foreach ($remap in $remaps) {
        if ($remap[0]) {
            $flags += "--remap-path-prefix=$($remap[0])=$($remap[1])"
            $flags += "--remap-path-prefix=$($remap[0].Replace('\','/'))=$($remap[1])"
        }
    }
    $env:CARGO_ENCODED_RUSTFLAGS = $flags -join [char]31
    if ($Test) {
        & $cargo "+$toolchain" test --locked
        if ($LASTEXITCODE -ne 0) { throw 'Rust tests failed.' }
    }
    & $cargo "+$toolchain" build --release --locked --bin dexdesk
    if ($LASTEXITCODE -ne 0) { throw 'Rust release build failed.' }
    New-Item -ItemType Directory -Path $OutputDirectory -Force | Out-Null
    $exe = Join-Path (Resolve-Path -LiteralPath $OutputDirectory) 'DeXDesk.exe'
    Copy-Item -LiteralPath (Join-Path $PSScriptRoot 'target\release\dexdesk.exe') `
        -Destination $exe -Force
    & $cargo "+$toolchain" run --locked --bin resource_stamp -- $exe (Join-Path $PSScriptRoot 'app.manifest')
    if ($LASTEXITCODE -ne 0) { throw 'Windows application resource packaging failed.' }
    $latin = [Text.Encoding]::GetEncoding(28591)
    $binary = $latin.GetString([IO.File]::ReadAllBytes($exe))
    foreach ($remap in $remaps) {
        if (-not $remap[0]) { continue }
        foreach ($path in @($remap[0],$remap[0].Replace('\','/'))) {
            foreach ($encoding in @([Text.Encoding]::UTF8,[Text.Encoding]::Unicode)) {
                $pattern = $latin.GetString($encoding.GetBytes($path))
                if ($binary.IndexOf($pattern,[StringComparison]::OrdinalIgnoreCase) -ge 0) {
                    throw 'Release privacy check failed: the EXE still contains a local build path.'
                }
            }
        }
    }
    Get-FileHash -LiteralPath $exe -Algorithm SHA256
}
finally {
    Pop-Location
    $env:CARGO_HOME=$previous.CARGO_HOME
    $env:RUSTUP_HOME=$previous.RUSTUP_HOME
    $env:PATH=$previous.PATH
    $env:CARGO_ENCODED_RUSTFLAGS=$previous.CARGO_ENCODED_RUSTFLAGS
}
