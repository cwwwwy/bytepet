# ---------------------------------------------------------------------------
# REFERENCE TEMPLATE — not the runtime script.
#
# `bytepet hooks install codex` generates <data_dir>/hooks/codex-notify.ps1 plus a
# codex-notify.cmd launcher, with __PET_PORT__ / __PET_CHAIN__ filled in.
# ~/.codex/config.toml then points at the .cmd wrapper on Windows:
#
#   notify = ["<data_dir>\\hooks\\codex-notify.cmd"]
#
# Codex passes the notify JSON as the last argument; the wrapper also accepts it
# on stdin. It forwards an AgentEvent, chains to the previous notify command
# and always exits 0.
# ---------------------------------------------------------------------------
$ErrorActionPreference = 'SilentlyContinue'
$PetPort = __PET_PORT__
$PetSource = 'codex'
$PetStateUrl = "http://127.0.0.1:$PetPort/state"
$PetChain = @(
__PET_CHAIN__
)

function Pet-Field($json, $name) {
  if ([string]::IsNullOrWhiteSpace($json)) { return '' }
  try { $obj = $json | ConvertFrom-Json } catch { return '' }
  if ($null -eq $obj) { return '' }
  $prop = $obj.PSObject.Properties[$name]
  if ($null -eq $prop -or $null -eq $prop.Value) { return '' }
  return [string]$prop.Value
}

function Pet-Post($state, $message) {
  if ([string]::IsNullOrWhiteSpace($state)) { return }
  $body = @{ source = $PetSource; state = $state }
  if (-not [string]::IsNullOrWhiteSpace($message)) { $body['message'] = $message }
  try {
    Invoke-RestMethod -Uri $PetStateUrl -Method Post -ContentType 'application/json' `
      -Body ($body | ConvertTo-Json -Compress) -TimeoutSec 2 | Out-Null
  } catch { }
}

$petPayload = ''
try { if ([Console]::IsInputRedirected) { $petPayload = [Console]::In.ReadToEnd() } } catch { }
if ([string]::IsNullOrWhiteSpace($petPayload) -and $args.Count -gt 0) { $petPayload = [string]$args[-1] }

$petState = ''
if (-not [string]::IsNullOrWhiteSpace($petPayload)) {
  $petType = (Pet-Field $petPayload 'type').ToLowerInvariant()
  switch -Wildcard ($petType) {
    '*session-start*' { $petState = 'waving' }
    '*turn-complete*' { $petState = 'review' }
    '*turn-ended*'    { $petState = 'review' }
    '*complete*'      { $petState = 'review' }
    '*turn-start*'    { $petState = 'running' }
    '*start*'         { $petState = 'running' }
    '*prompt*'        { $petState = 'running' }
    '*notification*'  { $petState = 'waiting' }
    '*notify*'        { $petState = 'waiting' }
    '*approval*'      { $petState = 'waiting' }
    '*fail*'          { $petState = 'failed' }
    '*error*'         { $petState = 'failed' }
  }
}
if (-not [string]::IsNullOrWhiteSpace($petState)) {
  $petMessage = Pet-Field $petPayload 'last-assistant-message'
  if ([string]::IsNullOrWhiteSpace($petMessage)) { $petMessage = Pet-Field $petPayload 'message' }
  if ($petMessage.Length -gt 200) { $petMessage = $petMessage.Substring(0, 200) }
  Pet-Post $petState $petMessage
}

if ($PetChain.Count -gt 0) {
  $petRest = @()
  if ($PetChain.Count -gt 1) { $petRest = $PetChain[1..($PetChain.Count - 1)] }
  try { & $PetChain[0] @petRest @args *> $null } catch { }
}
exit 0
