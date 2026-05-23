use axum::{
    extract::{DefaultBodyLimit, Multipart, Query, State},
    response::Html,
    routing::{get, post},
    Json, Router,
};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use dotenvy::dotenv;
use tokio::fs;
use std::path::{Path, PathBuf};
use base64::{Engine as _, engine::general_purpose};
use serde_json::{json, Value};

#[derive(Serialize, Deserialize, Debug, Clone)]
struct AnalysisResult {
    item_name: String,
    item_id: String,
    original_price: Option<i32>,
    discount_amount: Option<i32>,
    sale_price: i32,
    discount_start: Option<String>,
    discount_end: Option<String>,
    price_tag_type: String,
    stock_status: String,
}

struct AppState {
    db: Pool<Postgres>,
    gemini_api_key: String,
    storage_path: String,
    processed_path: String,
    inbox_path: String,
    error_path: String,
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    tracing_subscriber::fmt::init();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let gemini_api_key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY must be set");
    let storage_path = std::env::var("STORAGE_PATH").unwrap_or_else(|_| "./storage".to_string());
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());

    let processed_path = std::env::var("PROCESSED_PATH")
        .unwrap_or_else(|_| format!("{}/processed", storage_path));
    let inbox_path = std::env::var("INBOX_PATH")
        .unwrap_or_else(|_| format!("{}/inbox", storage_path));
    let error_path = std::env::var("ERROR_PATH")
        .unwrap_or_else(|_| format!("{}/error", storage_path));

    fs::create_dir_all(&storage_path).await.expect("Failed to create storage directory");
    fs::create_dir_all(&processed_path).await.expect("Failed to create processed directory");
    fs::create_dir_all(&inbox_path).await.expect("Failed to create inbox directory");
    fs::create_dir_all(&error_path).await.expect("Failed to create error directory");

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to Postgres");

    sqlx::query("ALTER TABLE costco_items ADD COLUMN IF NOT EXISTS product_image_url TEXT")
        .execute(&pool)
        .await
        .ok();

    let shared_state = Arc::new(AppState {
        db: pool,
        gemini_api_key,
        storage_path,
        processed_path,
        inbox_path: inbox_path.clone(),
        error_path,
    });

    // 폴더 감시 태스크 시작
    let watcher_state = shared_state.clone();
    tokio::spawn(async move {
        watch_inbox_folder(watcher_state).await;
    });

    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/upload", post(upload_handler))
        .route("/items", get(items_handler))
        .route("/admin", get(admin_page_handler))
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024))
        .with_state(shared_state);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    tracing::info!("inbox folder watching: {}", inbox_path);
    axum::serve(listener, app).await.unwrap();
}

// inbox 폴더를 감시하다가 새 파일이 생기면 자동 처리
async fn watch_inbox_folder(state: Arc<AppState>) {
    let (tx, mut rx) = tokio::sync::mpsc::channel::<PathBuf>(32);

    let mut watcher = RecommendedWatcher::new(
        move |res: Result<Event, _>| {
            if let Ok(event) = res {
                if matches!(event.kind, EventKind::Create(_)) {
                    for path in event.paths {
                        let _ = tx.blocking_send(path);
                    }
                }
            }
        },
        notify::Config::default(),
    )
    .expect("Failed to create file watcher");

    watcher
        .watch(Path::new(&state.inbox_path), RecursiveMode::NonRecursive)
        .expect("Failed to watch inbox folder");

    tracing::info!("Started watching inbox folder: {}", state.inbox_path);

    while let Some(path) = rx.recv().await {
        // 이미지 파일만 처리
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
        if !["jpg", "jpeg", "png", "webp"].contains(&ext.as_str()) {
            continue;
        }

        tracing::info!("New file detected in inbox: {:?}", path);

        // 파일이 완전히 쓰여질 때까지 잠시 대기
        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

        let state = state.clone();
        tokio::spawn(async move {
            const MAX_RETRIES: u32 = 3;
            let mut last_error = String::new();

            for attempt in 1..=MAX_RETRIES {
                match fs::read(&path).await {
                    Ok(data) => {
                        let file_name = path.file_name().unwrap().to_string_lossy().to_string();
                        match process_image(&state, data, file_name, Some(path.clone())).await {
                            Ok((detected, saved, _)) => {
                                tracing::info!(
                                    "Inbox processed (attempt {}/{}): detected={}, saved={}",
                                    attempt, MAX_RETRIES, detected, saved
                                );
                                return;
                            }
                            Err(e) => {
                                last_error = e.to_string();
                                tracing::warn!(
                                    "Processing failed (attempt {}/{}): {}",
                                    attempt, MAX_RETRIES, e
                                );
                            }
                        }
                    }
                    Err(e) => {
                        last_error = e.to_string();
                        tracing::warn!(
                            "Failed to read file (attempt {}/{}): {}",
                            attempt, MAX_RETRIES, e
                        );
                    }
                }

                if attempt < MAX_RETRIES {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                }
            }

            // 3번 모두 실패 → error 폴더로 이동
            tracing::error!(
                "All {} attempts failed for {:?}: {}",
                MAX_RETRIES, path, last_error
            );
            let error_dest = Path::new(&state.error_path)
                .join(path.file_name().unwrap());
            if let Err(e) = fs::rename(&path, &error_dest).await {
                tracing::error!("Failed to move to error folder: {}", e);
            }
        });
    }
}

// HTTP 업로드 핸들러
async fn upload_handler(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Json<serde_json::Value> {
    let mut file_data = Vec::new();
    let mut file_name = String::new();

    while let Some(field) = multipart.next_field().await.unwrap() {
        let name = field.name().unwrap().to_string();
        if name == "image" {
            file_name = field.file_name().unwrap_or("image.jpg").to_string();
            file_name = format!("{}_{}", chrono::Utc::now().timestamp(), file_name);
            file_data = field.bytes().await.unwrap().to_vec();
        }
    }

    if file_data.is_empty() {
        return Json(json!({ "error": "No image uploaded" }));
    }

    tracing::info!("Upload received: {} bytes, file: {}", file_data.len(), file_name);
    match process_image(&state, file_data, file_name, None).await {
        Ok((detected, saved, items)) => Json(json!({
            "status": "success",
            "detected_count": detected,
            "saved_count": saved,
            "items": items,
        })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

// 날짜별 아이템 조회 API
async fn items_handler(
    State(state): State<Arc<AppState>>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> Json<serde_json::Value> {
    let date = params.get("date").cloned()
        .unwrap_or_else(|| chrono::Utc::now().format("%Y-%m-%d").to_string());

    let rows = sqlx::query_as::<_, (i32, String, String, Option<i32>, Option<i32>, i32, Option<String>, Option<String>, String, String, Option<String>, Option<chrono::DateTime<chrono::Utc>>, Option<String>)>(
        r#"SELECT DISTINCT ON (item_id)
                  idx, item_id, item_name, original_price, discount_amount, sale_price,
                  discount_start::text, discount_end::text, price_tag_type, stock_status, image_url, uploaded_at, product_image_url
           FROM costco_items
           WHERE DATE(uploaded_at AT TIME ZONE 'Asia/Seoul') = $1::date
           ORDER BY item_id, idx DESC"#,
    )
    .bind(&date)
    .fetch_all(&state.db)
    .await;

    match rows {
        Ok(items) => {
            let list: Vec<_> = items.iter().map(|r| json!({
                "idx": r.0,
                "item_id": r.1,
                "item_name": r.2,
                "original_price": r.3,
                "discount_amount": r.4,
                "sale_price": r.5,
                "discount_start": r.6,
                "discount_end": r.7,
                "price_tag_type": r.8,
                "stock_status": r.9,
                "image_url": r.10,
                "uploaded_at": r.11,
                "product_image_url": r.12,
            })).collect();
            Json(json!({ "date": date, "count": list.len(), "items": list }))
        }
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

// 웹 관리 페이지
async fn admin_page_handler() -> Html<&'static str> {
    Html(include_str!("admin.html"))
}

// 공통 이미지 처리 함수
// source_path: inbox에서 온 경우 원본 경로(이동 후 삭제), None이면 storage에 직접 저장
async fn process_image(
    state: &AppState,
    file_data: Vec<u8>,
    file_name: String,
    source_path: Option<PathBuf>,
) -> Result<(usize, usize, Vec<AnalysisResult>), Box<dyn std::error::Error + Send + Sync>> {
    let uploaded_at = chrono::Utc::now();
    let date_str = uploaded_at.format("%Y-%m-%d").to_string();

    // 날짜별 processed 폴더
    let processed_date_dir = Path::new(&state.processed_path).join(&date_str);
    let processed_file_path = processed_date_dir.join(&file_name);

    // HTTP 업로드인 경우 storage에 임시 저장
    let temp_path = if source_path.is_none() {
        let p = Path::new(&state.storage_path).join(&file_name);
        fs::write(&p, &file_data).await?;
        Some(p)
    } else {
        None
    };

    // Gemini AI 분석
    let analysis_results = analyze_with_gemini(&state.gemini_api_key, &file_data).await?;

    // DB 저장
    let mut saved_count = 0;
    let discount_start_vals: Vec<_> = analysis_results.iter()
        .map(|i| i.discount_start.as_ref().and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()))
        .collect();
    let discount_end_vals: Vec<_> = analysis_results.iter()
        .map(|i| i.discount_end.as_ref().and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()))
        .collect();

    for (idx, item) in analysis_results.iter().enumerate() {
        let product_img = fetch_costco_image(&item.item_id).await;

        // 같은 날 같은 item_id가 있으면 UPDATE, 없으면 INSERT
        let updated = sqlx::query(
            r#"
            UPDATE costco_items SET
                item_name = $2, original_price = $3, discount_amount = $4, sale_price = $5,
                discount_start = $6, discount_end = $7, price_tag_type = $8, stock_status = $9,
                image_url = $10, uploaded_at = $11, product_image_url = $12
            WHERE item_id = $1
              AND DATE(uploaded_at AT TIME ZONE 'Asia/Seoul') = DATE($11 AT TIME ZONE 'Asia/Seoul')
            "#,
        )
        .bind(&item.item_id)
        .bind(&item.item_name)
        .bind(item.original_price)
        .bind(item.discount_amount)
        .bind(item.sale_price)
        .bind(discount_start_vals[idx])
        .bind(discount_end_vals[idx])
        .bind(&item.price_tag_type)
        .bind(&item.stock_status)
        .bind(processed_file_path.to_str())
        .bind(uploaded_at)
        .bind(product_img.as_deref())
        .execute(&state.db)
        .await;

        let done = match updated {
            Ok(r) if r.rows_affected() > 0 => true,
            _ => {
                sqlx::query(
                    r#"
                    INSERT INTO costco_items
                    (item_id, item_name, original_price, discount_amount, sale_price, discount_start, discount_end, price_tag_type, stock_status, image_url, uploaded_at, product_image_url)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
                    "#,
                )
                .bind(&item.item_id)
                .bind(&item.item_name)
                .bind(item.original_price)
                .bind(item.discount_amount)
                .bind(item.sale_price)
                .bind(discount_start_vals[idx])
                .bind(discount_end_vals[idx])
                .bind(&item.price_tag_type)
                .bind(&item.stock_status)
                .bind(processed_file_path.to_str())
                .bind(uploaded_at)
                .bind(product_img.as_deref())
                .execute(&state.db)
                .await
                .is_ok()
            }
        };

        if done { saved_count += 1; }
    }

    // DB 저장 성공 시 날짜 폴더로 이동
    if saved_count > 0 {
        fs::create_dir_all(&processed_date_dir).await?;

        let from = source_path.as_deref()
            .or(temp_path.as_deref())
            .unwrap();

        if let Err(e) = fs::rename(from, &processed_file_path).await {
            tracing::warn!("Failed to move file to processed folder: {}", e);
        }
    }

    Ok((analysis_results.len(), saved_count, analysis_results))
}

async fn fetch_costco_image(item_id: &str) -> Option<String> {
    let url = format!(
        "https://www.costco.co.kr/rest/v2/korea/products/{}?fields=images,code&lang=ko&curr=KRW",
        item_id
    );
    let client = reqwest::Client::new();
    let res = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
        .header("Accept", "application/json")
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .ok()?;
    let json: serde_json::Value = res.json().await.ok()?;
    let images = json.get("images")?.as_array()?;
    let relative = images.first()?.get("url")?.as_str()?;
    Some(format!("https://www.costco.co.kr{}", relative))
}

async fn analyze_with_gemini(api_key: &str, image_data: &[u8]) -> Result<Vec<AnalysisResult>, Box<dyn std::error::Error + Send + Sync>> {
    let base64_image = general_purpose::STANDARD.encode(image_data);
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-2.5-flash-lite:generateContent?key={}",
        api_key
    );

    let prompt = "
    You are a professional Costco price tag analyzer.
    Analyze the provided image and extract information for ALL price tags visible.
    Return a JSON array of objects with these fields:
    1. item_name: Product name (Korean and English).
    2. item_id: 6-7 digit product number.
    3. original_price: Integer (if visible).
    4. discount_amount: Integer (if visible).
    5. sale_price: Final price as integer.
    6. discount_start: YYYY-MM-DD (if visible).
    7. discount_end: YYYY-MM-DD (if visible).
    8. price_tag_type based on last 2 digits of sale_price:
       - 'Normal' if ends in 90
       - 'Double Discount' if ends in 70 or 00
       - 'Manufacturer Discount' if ends in 49 or 79
       - 'Normal' otherwise
    9. stock_status: '+' -> 'In Stock', '*' -> 'Last Chance', else 'Normal'.

    Respond ONLY with a valid JSON array. If nothing found, return [].
    ";

    let body = json!({
        "contents": [{
            "parts": [
                { "text": prompt },
                {
                    "inline_data": {
                        "mime_type": "image/jpeg",
                        "data": base64_image
                    }
                }
            ]
        }]
    });

    let client = reqwest::Client::new();
    let res = client.post(url).json(&body).send().await?;
    let response_json: Value = res.json().await?;

    let text = response_json["candidates"][0]["content"]["parts"][0]["text"]
        .as_str()
        .ok_or("Invalid response from Gemini")?;

    tracing::info!("Gemini raw response: {}", &text[..text.len().min(500)]);

    let json_str = text.trim_matches(|c| c == '`' || c == '\n' || c == ' ');
    let json_str = if json_str.starts_with("json") { &json_str[4..] } else { json_str };

    let results: Vec<AnalysisResult> = serde_json::from_str(json_str).map_err(|e| {
        tracing::error!("JSON parse error: {} — raw: {}", e, &json_str[..json_str.len().min(300)]);
        e
    })?;
    tracing::info!("Gemini parsed {} items", results.len());
    Ok(results)
}
