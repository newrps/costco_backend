# 배포 가이드

## 스토리지 구조

```
storage/
  inbox/              ← 사진/영상을 여기에 넣으면 자동 처리
  processed/
    2026-05-25/       ← DB 저장 성공 시 날짜 폴더로 이동
  error/              ← 3회 재시도 후 실패한 파일
  product_images/     ← 앱에서 직접 찍어 올린 상품 사진 ({item_id}.jpg)
```

---

## 최초 NAS 세팅 (처음 한 번만)

### 1. NAS에 폴더 생성

SSH 접속:
```bash
ssh -p 56822 newrps@192.168.123.110
```

프로젝트 폴더 및 스토리지 생성:
```bash
mkdir -p /volume1/docker/costco_backend/storage/inbox
mkdir -p /volume1/docker/costco_backend/storage/processed
mkdir -p /volume1/docker/costco_backend/storage/error
mkdir -p /volume1/docker/costco_backend/storage/product_images
```

### 2. NAS에 .env 파일 생성

> **보안 주의**: GEMINI_API_KEY는 절대 채팅창이나 터미널에 붙여넣지 말 것. nano로 직접 입력.

```bash
nano /volume1/docker/costco_backend/.env
```

내용:
```
DATABASE_URL=postgres://newrps:Pspspsps1234!!@db:5432/costco_db
GEMINI_API_KEY=여기에_Gemini_API_키_입력
STORAGE_PATH=/app/storage
INBOX_PATH=/app/storage/inbox
PROCESSED_PATH=/app/storage/processed
ERROR_PATH=/app/storage/error
PRODUCT_IMAGES_PATH=/app/storage/product_images
BASE_URL=https://zip.zam.kr
PORT=3000
```

저장: `Ctrl+X` → `Y` → `Enter`

### 3. 최초 배포

Windows PC 프로젝트 폴더에서:
```powershell
.\push-deploy.ps1 -Force
```

---

## 이후 배포 (코드 변경 시)

### 기본 배포 (커밋 + GitHub 푸시 + NAS 배포 한 번에)

```powershell
.\push-deploy.ps1
```

### 옵션

```powershell
# 커밋 메시지 직접 지정
.\push-deploy.ps1 -Message "기능 추가"

# 변경사항 없어도 강제 재배포
.\push-deploy.ps1 -Force

# git 커밋/푸시 없이 배포만
.\push-deploy.ps1 -SkipGit -Force
```

### 배포 과정 (자동)

1. `git add` + `git commit` + `git push`
2. 소스 tar로 압축 → SCP로 NAS 전송
3. NAS에서 압축 해제 → `docker compose up -d --build`

> 빌드 시간: 처음 2~5분, 이후 캐시 활용으로 1~2분

---

## 가격표 처리 방법

### 방법 1 - 관리자 페이지 업로드

`http://192.168.123.110:3100/admin` → 파일 선택 → 업로드

### 방법 2 - HTTP API

```bash
curl -X POST http://192.168.123.110:3100/upload \
  -F "image=@/path/to/photo.jpg"
```

### 방법 3 - inbox 폴더 (자동 감지)

```bash
scp -P 56822 photo.jpg newrps@192.168.123.110:/volume1/docker/costco_backend/storage/inbox/
```

파일이 감지되면 자동으로:
1. Gemini AI 분석 (현재 모델: `gemini-2.5-flash-lite`)
2. DB 저장 (같은 날 같은 item_id → UPDATE, 아니면 INSERT)
3. `processed/날짜/` 폴더로 이동
4. 실패 시 1초 간격 최대 3회 재시도
5. 3회 모두 실패 → `error/` 폴더로 이동

---

## 기존 DB에 컬럼 추가 (DB가 이미 있는 경우)

init.sql은 최초 컨테이너 생성 시에만 실행됨. 이미 DB가 있다면 수동으로 추가:

```bash
/usr/local/bin/docker exec costco-db psql -U newrps -d costco_db
```

```sql
ALTER TABLE costco_items ADD COLUMN IF NOT EXISTS uploaded_at TIMESTAMPTZ;
ALTER TABLE costco_items ADD COLUMN IF NOT EXISTS product_image_url TEXT;
ALTER TABLE costco_items ADD COLUMN IF NOT EXISTS category TEXT;
-- 상품 사진 컬럼은 product_image_url을 재사용 (별도 컬럼 없음)
\q
```

---

## 로그 확인

```bash
ssh -p 56822 newrps@192.168.123.110

# 실시간 로그
/usr/local/bin/docker logs -f costco-backend

# 최근 100줄
/usr/local/bin/docker logs --tail 100 costco-backend

# 에러만 필터
/usr/local/bin/docker logs costco-backend 2>&1 | grep ERROR
```

## 서비스 관리

```bash
cd /volume1/docker/costco_backend

# 상태 확인
/usr/local/bin/docker compose ps

# 백엔드 재시작
/usr/local/bin/docker compose restart backend

# 전체 재시작
/usr/local/bin/docker compose down && /usr/local/bin/docker compose up -d
```
