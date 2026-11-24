param(
    [string]$ProcessName = "seu-navegador-spike",
    [int]$IntervalSeconds = 2,
    [int]$Samples = 0
)

Write-Host "=== Medidor de RAM - $ProcessName ===" -ForegroundColor Cyan
Write-Host "Pressione Ctrl+C para encerrar.`n"

$count = 0
while ($true) {
    $procs = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue
    $timestamp = Get-Date -Format "HH:mm:ss"

    if (-not $procs) {
        Write-Host "[$timestamp] Nenhum processo '$ProcessName' em execucao..." -ForegroundColor Yellow
    } else {
        $totalWS = ($procs | Measure-Object -Property WorkingSet64 -Sum).Sum / 1MB
        $totalPM = ($procs | Measure-Object -Property PrivateMemorySize64 -Sum).Sum / 1MB

        Write-Host ("[$timestamp] Processos: {0} | Working Set Total: {1:N1} MB | Private Bytes Total: {2:N1} MB" -f $procs.Count, $totalWS, $totalPM) -ForegroundColor Green

        $procs | Sort-Object WorkingSet64 -Descending | ForEach-Object {
            $ws = $_.WorkingSet64 / 1MB
            $pm = $_.PrivateMemorySize64 / 1MB
            Write-Host ("    PID {0,-7} | WS: {1,7:N1} MB | Private: {2,7:N1} MB" -f $_.Id, $ws, $pm)
        }
        Write-Host ""
    }

    $count++
    if ($Samples -gt 0 -and $count -ge $Samples) {
        break
    }

    Start-Sleep -Seconds $IntervalSeconds
}
