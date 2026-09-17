# Build sauron, serve the board, and open it in a browser.
#
# The browser is where the agents run now -- `serve` opens a pty per agent and
# the page is the terminal on the end of it. Closing the tab does not stop them;
# Ctrl-C here does.
#
#   .\run.ps1                    watch the repo you are standing in
#   .\run.ps1 C:\path\to\repo    watch a specific one
#   .\run.ps1 --port 8080        bind somewhere particular
#   .\run.ps1 --agents 0         serve without reopening in-flight sessions
#   .\run.ps1 --no-open          serve without launching a browser
#   .\run.ps1 --tui              the terminal front end instead, no server
#
# Anything else is passed through to sauron, so `--codex`, `--bind`, and the
# rest still work. This is the PowerShell twin of run.sh; the two keep the same
# flags and the same behaviour so the README's commands read the same on both.
#
# WHY THIS WAITS BEFORE IT OPENS
# ------------------------------
# A browser sent to a port nothing is listening to yet lands on a
# connection-refused page, and it will not retry. So the script polls until the
# server answers, and only then hands the URL over. A fixed sleep fails on a
# cold cargo build and looks like a bug in sauron rather than in the launcher.
#
# WHY THE PORT MAY NOT BE THE ONE YOU ASKED FOR
# ---------------------------------------------
# sauron is normally run several at a time, one per repo, so a busy default port
# is the common case and not an error. With no explicit --port, the script walks
# upward from the default until it finds a free one and prints which it took. An
# explicit --port is a request, not a hint, and a clash there is reported.

# Print the usage block from this file's own header, the way run.sh does.
function Show-Help {
    Get-Content -LiteralPath $PSCommandPath |
        Select-Object -Skip 6 -First 6 |
        ForEach-Object { $_ -replace '^#\s?', '' }
}

$DefaultPort = 7373
$Port = ''
$Open = $true
$Tui = $false
$Repo = ''
$Pass = @()

# run.sh's argument loop, kept token-for-token so the two launchers agree. A
# lone `--*` flag other than --port passes straight through to sauron; the first
# bare word is the repo.
for ($i = 0; $i -lt $args.Count; $i++) {
    $arg = [string]$args[$i]
    if ($arg -eq '--port') {
        $Port = [string]$args[$i + 1]; $i++
    } elseif ($arg -like '--port=*') {
        $Port = $arg.Substring(7)
    } elseif ($arg -eq '--no-open') {
        $Open = $false
    } elseif ($arg -eq '--tui') {
        $Tui = $true
    } elseif ($arg -eq '-h' -or $arg -eq '--help') {
        Show-Help; exit 0
    } elseif ($arg -like '--*') {
        $Pass += $arg
    } else {
        $Repo = $arg
    }
}

# Where the script lives, so it can run from anywhere; and where it was run
# from, which is the repo the user means unless they named one.
$Here = $PSScriptRoot
$From = (Get-Location).Path
$Bin = Join-Path $Here 'sauron\target\release\sauron.exe'

Write-Host 'building...'
cargo build --release --manifest-path (Join-Path $Here 'sauron\Cargo.toml')
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

Set-Location $From
if ($Repo -ne '') { $Pass += $Repo }

if ($Tui) {
    & $Bin @Pass
    exit $LASTEXITCODE
}

# Free-port probe. A refused connection means the port is free; a connection
# that lands means something already holds it. .NET's TcpClient needs nothing
# installed, which is the point on a fresh Windows.
function Test-PortFree([int]$p) {
    $client = New-Object System.Net.Sockets.TcpClient
    try {
        $client.Connect('127.0.0.1', $p)
        $client.Close()
        return $false
    } catch {
        return $true
    }
}

if ($Port -eq '') {
    $Port = $DefaultPort
    while (-not (Test-PortFree $Port)) {
        $Port = $Port + 1
        if ($Port -gt ($DefaultPort + 20)) {
            Write-Error "run.ps1: no free port in $DefaultPort..$($DefaultPort + 20)"
            exit 1
        }
    }
    if ($Port -ne $DefaultPort) {
        Write-Host "run.ps1: $DefaultPort was busy, taking $Port"
    }
} elseif (-not (Test-PortFree ([int]$Port))) {
    # Asked for, not available. Say so here rather than letting sauron fail its
    # bind, because the person who typed a port wants to know it was taken.
    Write-Error "run.ps1: port $Port is already in use"
    exit 1
}

$ServerArgs = @('serve', '--port', "$Port") + $Pass
$Server = Start-Process -FilePath $Bin -ArgumentList $ServerArgs -NoNewWindow -PassThru
$Url = "http://127.0.0.1:$Port"

try {
    # Poll rather than sleep. Two seconds of tries at 100ms, far longer than a
    # bind takes and short enough that a server that died on startup is reported
    # promptly instead of hanging the launcher.
    $Ready = $false
    for ($i = 0; $i -lt 20; $i++) {
        if ($Server.HasExited) {
            Write-Error 'run.ps1: sauron exited before it was listening'
            exit 1
        }
        if (-not (Test-PortFree ([int]$Port))) { $Ready = $true; break }
        Start-Sleep -Milliseconds 100
    }

    if (-not $Ready) {
        Write-Error "run.ps1: gave up waiting for $Url"
        exit 1
    }

    if ($Open) {
        Start-Process $Url
    }

    Write-Host "run.ps1: $Url -- Ctrl-C here stops sauron and every agent it opened"
    $Server.WaitForExit()
} finally {
    # Ctrl-C, or the script ending, takes the server with it. Without this a
    # detached sauron would outlive the launcher and hold the port.
    if ($Server -and -not $Server.HasExited) {
        $Server.Kill()
    }
}
