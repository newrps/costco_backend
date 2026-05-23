# 코스트코 가격표 백엔드 운영 가이드

## 서비스 개요

코스트코 매장에서 가격표 사진을 찍으면 Gemini AI가 자동으로 분석해서 DB에 저장하는 시스템.

- **서버 주소**: `http://192.168.123.110:3100`
- **관리자 페이지**: `http://192.168.123.110:3100/admin`
- **운영 서버**: NAS (Synology, 192.168.123.110)

---

## 1. 코드 수정 후 배포

### 한 번에 배포 (커밋 + GitHub 푸시 + NAS 배포)

```powershell
.\push-deploy.ps1
```

커밋 메시지 직접 지정:
```powershell
.\push-deploy.ps1 -Message "상품 이미지 기능 추가"
```

코드 변경 없이 강제 재배포:
```powershell
.\push-deploy.ps1 -Force
```

git 없이 NAS 배포만:
```powershell
.\push-deploy.ps1 -SkipGit -Force
```

> 빌드 시간: 처음 2~5분, 이후 캐시 활용으로 단축

---

## 2. OpenAI API 키 교체

키가 만료되거나 교체가 필요하면 로그에 `OpenAI API error` 표시.

### 새 키 발급
[https://platform.openai.com/api-keys](https://platform.openai.com/api-keys) → **Create new secret key**

### NAS에 직접 교체 (절대 채팅/터미널에 키를 붙여넣지 말 것)

**1. SSH 접속**
```powershell
ssh -p 56822 newrps@192.168.123.110
```

**2. .env 파일 수정**
```bash
nano /volume1/docker/costco_backend/.env
```

`OPENAI_API_KEY=` 줄을 새 키로 교체 후 저장 (`Ctrl+X` → `Y` → `Enter`)

**3. 백엔드 재시작**
```bash
cd /volume1/docker/costco_backend
/usr/local/bin/docker compose restart backend
```

---

## 3. 가격표 등록 방법

### 방법 1 - 앱/API로 직접 업로드
```
POST http://192.168.123.110:3100/upload
Content-Type: multipart/form-data
필드: image (파일)
```

### 방법 2 - inbox 폴더에 사진 복사 (자동 감지)
```powershell
scp -P 56822 photo.jpg newrps@192.168.123.110:/volume1/docker/costco_backend/storage/inbox/
```

파일 감지 후 자동 처리:
1. Gemini AI 이미지 분석
2. DB 저장 (같은 날 같은 상품이면 UPDATE, 아니면 INSERT)
3. `storage/processed/YYYY-MM-DD/` 폴더로 이동
4. 실패 시 1초 간격 최대 3회 재시도
5. 3회 모두 실패 → `storage/error/` 폴더로 이동

---

## 4. 가격표 종류 (price_tag_type)

| 끝 두 자리 | 종류 | 설명 |
|---|---|---|
| .90 | Normal | 일반 정상가 |
| .70 / .00 | Double Discount | 매니저 특별 할인 (재고 소진용 대폭 할인) |
| .49 / .79 | Manufacturer Discount | 생산업체 프로모션 할인 |

## 5. 재고 상태 (stock_status)

| 기호 | 상태 | 설명 |
|---|---|---|
| + | In Stock | 재입고 불분명 — 판매 추이에 따라 결정 |
| * | Last Chance | 마지막 물량 — 재주문 없이 소진 후 단종 |
| (없음) | Normal | 일반 재고 |

---

## 6. 로그 확인

```bash
ssh -p 56822 newrps@192.168.123.110

# 실시간 로그
/usr/local/bin/docker logs -f costco-backend

# 최근 100줄
/usr/local/bin/docker logs --tail 100 costco-backend
```

---

## 7. DB 직접 조회/관리

```bash
ssh -p 56822 newrps@192.168.123.110
/usr/local/bin/docker exec costco-db psql -U newrps -d costco_db
```

유용한 SQL:
```sql
-- 전체 데이터 확인
SELECT * FROM costco_items ORDER BY uploaded_at DESC LIMIT 20;

-- 오늘 데이터
SELECT * FROM costco_items WHERE DATE(uploaded_at AT TIME ZONE 'Asia/Seoul') = CURRENT_DATE;

-- 데이터 전체 삭제
TRUNCATE TABLE costco_items;

-- 종료
\q
```

---

## 8. 서비스 재시작 / 중지

```bash
ssh -p 56822 newrps@192.168.123.110
cd /volume1/docker/costco_backend

# 재시작
/usr/local/bin/docker compose restart backend

# 전체 중지
/usr/local/bin/docker compose down

# 전체 시작
/usr/local/bin/docker compose up -d
```

---

## 9. 최초 NAS 세팅 (처음 한 번만)

```bash
ssh -p 56822 newrps@192.168.123.110

mkdir -p /volume1/docker/costco_backend
cd /volume1/docker/costco_backend
```

`.env` 파일 생성:
```bash
nano .env
```

내용:
```
DATABASE_URL=postgres://newrps:Pspspsps1234!!@db:5432/costco_db
OPENAI_API_KEY=여기에_OpenAI_API_키_입력
PORT=3000
STORAGE_PATH=/app/storage
INBOX_PATH=/app/storage/inbox
PROCESSED_PATH=/app/storage/processed
ERROR_PATH=/app/storage/error
```

이후 Windows PC에서 첫 배포:
```powershell
.\push-deploy.ps1 -Force
```
