# 코스트코 가격표 관리 시스템

코스트코 매장에서 가격표 사진을 찍으면 Gemini AI가 자동으로 분석해 DB에 저장하고, 할인 정보를 조회·공유할 수 있는 시스템.

## 기술 스택

- **백엔드**: Rust (Axum)
- **DB**: PostgreSQL 15
- **AI 분석**: Google Gemini 2.5 Flash Lite
- **인프라**: Synology NAS, Docker Compose
- **배포**: PowerShell 스크립트 (`push-deploy.ps1`)

---

## 서비스 주소

| 페이지 | URL | 설명 |
|--------|-----|------|
| 공개 페이지 | `http://192.168.123.110:3100/` | 오늘의 전체 상품 목록 |
| 할인 페이지 | `http://192.168.123.110:3100/sale` | 할인 상품만 표시 |
| 상품 상세 | `http://192.168.123.110:3100/item/:id` | 가격 이력 차트 + 즐겨찾기 |
| 관리자 | `http://192.168.123.110:3100/admin` | 가격표 업로드 및 전체 관리 |

---

## 주요 기능

### 가격표 OCR
- 사진 또는 영상 업로드 → Gemini AI가 상품명·가격·할인 정보 자동 추출
- inbox 폴더 감시: NAS 폴더에 파일 복사하면 자동 처리
- 같은 날 동일 상품 재업로드 시 UPDATE (중복 방지)

### 상품 사진 관리
- 안드로이드 앱에서 분석 결과 1개짜리 항목에 "상품 사진 찍기" 버튼 노출
- `POST /product-image` 로 업로드 → `storage/product_images/{item_id}.jpg` 에 저장
- 해당 item_id의 모든 DB 레코드에 `product_image_url` 자동 업데이트
- `GET /product-images/:filename` 으로 직접 서빙 (저작권 문제 없는 UGC 이미지)

### 공개/할인 페이지
- 한국어 초성 검색 (`ㅂㄷ` → 불닭 등)
- 즐겨찾기 (localStorage 저장, 필터 가능)
- 상품 카드 클릭 → 상세 페이지 이동
- KakaoTalk 공유 시 OG 이미지 미리보기

### 상품 상세 페이지
- 전체 가격 이력 차트 (Canvas)
- 날짜별 가격 변화 테이블
- 즐겨찾기 토글

### 관리자 페이지
- 가격표 사진/영상 업로드
- 한국어 초성 검색 + 전체기간 검색
- 할인 상품 필터 (Double Discount / Clearance / Manufacturer Discount)
- 목록 복사 → 네이버 웹 에디터에 표로 붙여넣기
- 수동 조회 (자동 새로고침 없음)

### 가격표 종류

| 끝 두 자리 | 종류 | 의미 |
|-----------|------|------|
| 90 | Normal | 일반 정상가 |
| 70 / 00 | Double Discount | 재고 소진용 특별 할인 |
| 49 / 79 | Manufacturer Discount | 제조사 프로모션 할인 |

### 재고 상태

| 기호 | 상태 | 의미 |
|------|------|------|
| + | In Stock | 재입고 미정 |
| * | Last Chance | 마지막 물량, 단종 예정 |
| (없음) | Normal | 일반 재고 |

---

## API 엔드포인트

| 메서드 | 경로 | 설명 |
|--------|------|------|
| GET | `/` | 공개 페이지 |
| GET | `/sale` | 할인 페이지 |
| GET | `/admin` | 관리자 페이지 |
| GET | `/item/:id` | 상품 상세 페이지 |
| GET | `/item/:id/history` | 상품 가격 이력 (JSON) |
| POST | `/upload` | 가격표 이미지 업로드 |
| POST | `/product-image` | 상품 사진 업로드 (item_id 연결) |
| GET | `/product-images/:filename` | 업로드된 상품 사진 서빙 |
| GET | `/items?date=YYYY-MM-DD` | 날짜별 전체 상품 목록 |
| GET | `/sale-items` | 할인 상품 목록 |
| GET | `/search?q=검색어` | 전체기간 상품 검색 |
| GET | `/og-image.png` | KakaoTalk 공유 이미지 |

---

## 프로젝트 구조

```
costco_backend/
├── src/
│   ├── main.rs          # Rust 백엔드 (Axum 서버 + API)
│   ├── admin.html       # 관리자 페이지
│   ├── public.html      # 공개 페이지 (오늘 전체)
│   ├── sale.html        # 할인 상품 페이지
│   ├── item.html        # 상품 상세 페이지
│   ├── og-image.png     # KakaoTalk OG 이미지
│   └── favicon.svg      # 파비콘
├── Cargo.toml
├── docker-compose.yml
├── Dockerfile
├── init.sql             # DB 초기화 스크립트
├── push-deploy.ps1      # 배포 스크립트
├── GUIDE.md             # 운영 가이드
└── DEPLOY.md            # 배포·설치 가이드
```

---

## 빠른 배포

```powershell
# 코드 변경 후 커밋 + 배포
.\push-deploy.ps1

# 변경 없이 강제 재배포
.\push-deploy.ps1 -Force
```

자세한 내용은 [GUIDE.md](GUIDE.md), [DEPLOY.md](DEPLOY.md) 참조.
