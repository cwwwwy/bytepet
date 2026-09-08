# ---------------------------------------------------------------------------
# REFERENCE TEMPLATE — not the runtime script.
#
# `bytepet hooks install claude-code` generates <data_dir>/hooks/claude-hook.ps1
# plus a claude-hook.cmd launcher, with __PET_PORT__ filled in, and registers
# the .cmd as a command hook for SessionStart, UserPromptSubmit, PreToolUse,
# PostToolUse, Notification, Stop and SubagentStop.
#
# Claude Code sends the hook payload as JSON on stdin. The wrapper maps
# `hook_event_name` to a bytepet state, POSTs it, prints nothing and exits 0.
# ---------------------------------------------------------------------------
$ErrorActionPreference = 'SilentlyContinue'
$PetPort = __PET_PORT__
$PetSource = 'claude-code'
$PetStateUrl = "http://127.0.0.1:$PetPort/state"

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

$petState = ''
switch (Pet-Field $petPayload 'hook_event_name') {
  'SessionStart'     { $petState = 'waving' }
  'UserPromptSubmit' { $petState = 'running' }
  'PreToolUse'       { $petState = 'running' }
  'PostToolUse'      { $petState = 'running' }
  'Notification'     { $petState = 'waiting' }
  'Stop'             { $petState = 'review' }
  'SubagentStop'     { $petState = 'review' }
  'SessionEnd'       { $petState = 'review' }
}
if (-not [string]::IsNullOrWhiteSpace($petState)) {
  $petMessage = Pet-Field $petPayload 'message'
  if ([string]::IsNullOrWhiteSpace($petMessage)) { $petMessage = Pet-Field $petPayload 'last_assistant_message' }
  if ($petMessage.Length -gt 200) { $petMessage = $petMessage.Substring(0, 200) }
  Pet-Post $petState $petMessage
}
exit 0
