<#
.SYNOPSIS
  코스트코 백엔드 - git 커밋/푸시 + NAS 배포 스크립트

.USAGE
  .\push-deploy.ps1                        # 변경사항 커밋 + 배포
  .\push-deploy.ps1 -Message "기능 추가"   # 커밋 메시지 지정
  .\push-deploy.ps1 -Force                 # 변경 없어도 강제 재배포
  .\push-deploy.ps1 -SkipGit              # git 커밋/푸시 없이 배포만
#>

param(
  [string]$Message = "",
  [switch]$Force,
  [switch]$SkipGit
)

$ErrorActionPreference = "Continue"
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8

$nasUser = "newrps"
$nasHost = "192.168.123.110"
$nasPort = 56822
$nasPath = "/volume1/docker/costco_backend"

Write-Host "=== 코스트코 백엔드 배포 ===" -ForegroundColor Cyan

# 1) git 커밋 + 푸시
if (-not $SkipGit) {
  $status = git status --porcelain 2>$null
  if ($status) {
    $msg = if ($Message) { $Message } else { "update $(Get-Date -Format 'yyyy-MM-dd HH:mm')" }
    Write-Host "[git] 커밋: $msg" -ForegroundColor Cyan
    git add src/ Cargo.toml Cargo.lock docker-compose.yml init.sql push-deploy.ps1 GUIDE.md .gitignore 2>$null
    git commit -m $msg 2>$null | Out-Null
  } else {
    Write-Host "[git] 변경사항 없음" -ForegroundColor DarkGray
    if (-not $Force) {
      Write-Host "[git] 배포하려면 -Force 옵션 사용: .\push-deploy.ps1 -Force" -ForegroundColor Yellow
      exit 0
    }
  }

  $remote = git remote 2>$null
  if ($remote) {
    Write-Host "[git] GitHub 푸시 중..." -ForegroundColor Cyan
    git push 2>&1 | Out-Null
    if ($LASTEXITCODE -eq 0) {
      Write-Host "[git] 푸시 완료" -ForegroundColor Green
    } else {
      Write-Host "[git] 푸시 실패 (배포는 계속)" -ForegroundColor Yellow
    }
  }
}

# 2) git archive → tar → SCP
Write-Host "[nas] NAS로 전송 중..." -ForegroundColor Cyan
$tmpTar = Join-Path $env:TEMP ("costco_deploy_" + (Get-Random) + ".tar")
git archive HEAD --format=tar -o $tmpTar
if ($LASTEXITCODE -ne 0) {
  Write-Host "[nas] git archive 실패" -ForegroundColor Red
  exit 1
}

scp -O -P $nasPort -o StrictHostKeyChecking=no $tmpTar "${nasUser}@${nasHost}:${nasPath}/_deploy.tar" 2>$null
if ($LASTEXITCODE -ne 0) {
  Write-Host "[nas] SCP 전송 실패" -ForegroundColor Red
  Remove-Item $tmpTar -ErrorAction SilentlyContinue
  exit 1
}
Remove-Item $tmpTar -ErrorAction SilentlyContinue

# 3) NAS에서 압축 해제 + docker compose up --build
Write-Host "[nas] 빌드 및 재시작 중... (2~5분 소요)" -ForegroundColor Cyan
$remoteCmd = "cd '$nasPath' && rm -f docker-compose.yml && tar -xf _deploy.tar && rm -f _deploy.tar && /usr/local/bin/docker compose up -d --build"
ssh -p $nasPort -o StrictHostKeyChecking=no "${nasUser}@${nasHost}" $remoteCmd
if ($LASTEXITCODE -eq 0) {
  Write-Host "[완료] 배포 성공!" -ForegroundColor Green
  Write-Host "  관리자 페이지: http://${nasHost}:3100/admin" -ForegroundColor White
} else {
  Write-Host "[실패] docker compose 오류 발생" -ForegroundColor Red
  exit 1
}
