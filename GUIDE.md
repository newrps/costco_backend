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

## 2. Gemini API 키 교체

키가 만료되거나 교체가 필요하면 로그에 `Gemini API error 401` 또는 `403` 표시됨.

> **보안 주의**: API 키는 절대 채팅창이나 터미널 화면에 붙여넣지 말 것. 반드시 NAS에 SSH 접속 후 nano로 직접 수정.

### 새 키 발급
- **Cloud Console 키 (유료)**: [console.cloud.google.com](https://console.cloud.google.com) → API 및 서비스 → 사용자 인증 정보
- **AI Studio 키 (무료)**: [aistudio.google.com](https://aistudio.google.com) → Get API Key

### NAS에서 키 교체

**1. SSH 접속**
```powershell
ssh -p 56822 newrps@192.168.123.110
```

**2. .env 파일 수정**
```bash
nano /volume1/docker/costco_backend/.env
```

`GEMINI_API_KEY=` 줄을 새 키로 교체 후 저장 (`Ctrl+X` → `Y` → `Enter`)

**3. 백엔드 재시작**
```bash
cd /volume1/docker/costco_backend
/usr/local/bin/docker compose restart backend
```

### 현재 사용 모델
`gemini-2.5-flash-lite` (v1beta 엔드포인트)

| 모델 | 입력 (1M 토큰) | 출력 (1M 토큰) |
|------|--------------|--------------|
| gemini-2.5-flash-lite | $0.10 | $0.40 |
| gemini-2.5-flash | $0.30 | $2.50 |

---

## 3. 상품 사진 등록

앱에서 가격표 분석 결과가 **1개**일 때 "상품 사진 찍기" 버튼이 나타납니다.  
버튼 탭 → 상품을 카메라로 향해 촬영 → 자동 업로드.

### API 직접 호출
```bash
curl -X POST https://zip.zam.kr/product-image \
  -F "item_id=1234567" \
  -F "image=@/path/to/product.jpg"
```

**응답 예시**
```json
{ "status": "success", "image_url": "https://zip.zam.kr/product-images/1234567.jpg" }
```

- 저장 위치: `storage/product_images/{item_id}.jpg`
- DB의 해당 item_id 모든 레코드에 `product_image_url` 자동 업데이트
- 같은 item_id로 재업로드 시 파일 덮어쓰기 + DB 재업데이트

---

## 4. 가격표 등록 방법

### 방법 1 - 관리자 페이지에서 직접 업로드
`http://192.168.123.110:3100/admin` → 파일 선택 후 업로드

### 방법 2 - API로 업로드
```
POST http://192.168.123.110:3100/upload
Content-Type: multipart/form-data
필드: image (파일)
```

### 방법 3 - inbox 폴더에 사진 복사 (자동 감지)
```powershell
scp -P 56822 photo.jpg newrps@192.168.123.110:/volume1/docker/costco_backend/storage/inbox/
```

파일 감지 후 자동 처리:
1. Gemini AI 이미지 분석
2. DB 저장 (같은 날 같은 상품이면 UPDATE, 아니면 INSERT)
3. `storage/processed/YYYY-MM-DD/` 폴더로 이동
4. 실패 시 1초 간격 최대 3회 재시도
5. 3회 모두 실패 → `storage/error/` 폴더로 이동

### 사진 vs 영상 업로드 차이
- **사진**: 1장씩 분석 → 정확도 높음
- **영상**: Gemini가 여러 프레임을 동시에 분석 → 서로 다른 가격표 정보가 혼용될 수 있음
- 가능하면 **가격표 하나씩 사진으로 찍어 업로드**하는 것을 권장

---

## 5. 가격표 종류 (price_tag_type)

| 끝 두 자리 | 종류 | 설명 |
|-----------|------|------|
| .90 | Normal | 일반 정상가 |
| .70 / .00 | Double Discount | 매니저 특별 할인 (재고 소진용 대폭 할인) |
| .49 / .79 | Manufacturer Discount | 생산업체 프로모션 할인 |

## 6. 재고 상태 (stock_status)

| 기호 | 상태 | 설명 |
|------|------|------|
| + | In Stock | 재입고 불분명 — 판매 추이에 따라 결정 |
| * | Last Chance | 마지막 물량 — 재주문 없이 소진 후 단종 |
| (없음) | Normal | 일반 재고 |

---

## 7. 로그 확인

```bash
ssh -p 56822 newrps@192.168.123.110

# 실시간 로그
/usr/local/bin/docker logs -f costco-backend

# 최근 100줄
/usr/local/bin/docker logs --tail 100 costco-backend
```

---

## 8. DB 직접 조회/관리

```bash
ssh -p 56822 newrps@192.168.123.110
/usr/local/bin/docker exec costco-db psql -U newrps -d costco_db
```

유용한 SQL:
```sql
-- 전체 데이터 확인 (최신순)
SELECT * FROM costco_items ORDER BY uploaded_at DESC LIMIT 20;

-- 오늘 데이터
SELECT * FROM costco_items WHERE DATE(uploaded_at AT TIME ZONE 'Asia/Seoul') = CURRENT_DATE;

-- 특정 상품 이력
SELECT * FROM costco_items WHERE item_id = '123456' ORDER BY uploaded_at DESC;

-- 할인 상품만
SELECT * FROM costco_items WHERE discount_amount IS NOT NULL ORDER BY uploaded_at DESC;

-- 데이터 전체 삭제
TRUNCATE TABLE costco_items;

-- 종료
\q
```

---

## 9. 서비스 재시작 / 중지

```bash
ssh -p 56822 newrps@192.168.123.110
cd /volume1/docker/costco_backend

# 백엔드만 재시작
/usr/local/bin/docker compose restart backend

# 전체 중지
/usr/local/bin/docker compose down

# 전체 시작
/usr/local/bin/docker compose up -d

# 상태 확인
/usr/local/bin/docker compose ps
```

---

## 10. 자주 발생하는 문제

### Gemini API 오류: 모델 deprecated
```
Gemini API error 404: This model is no longer available
```
→ `src/main.rs`의 모델명을 최신 모델로 변경 후 재배포  
→ 현재 모델: `gemini-2.5-flash-lite`

### 분석은 됐는데 0개 저장
→ `storage/error/` 폴더 확인. JSON 파싱 실패 가능성.  
→ `docker logs costco-backend | grep ERROR` 로 원인 확인

### 영상 업로드 시 상품명/가격 혼용
→ 여러 가격표가 동시에 보이는 프레임에서 AI가 정보를 혼용하는 현상  
→ 가격표 하나씩 사진으로 찍어 업로드하면 해결

### DB 연결 오류
```bash
/usr/local/bin/docker compose restart db
/usr/local/bin/docker compose restart backend
```
