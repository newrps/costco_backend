use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
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
                            Ok((detected, saved)) => {
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

    match process_image(&state, file_data, file_name, None).await {
        Ok((detected, saved)) => Json(json!({
            "status": "success",
            "detected_count": detected,
            "saved_count": saved,
        })),
        Err(e) => Json(json!({ "error": e.to_string() })),
    }
}

// 공통 이미지 처리 함수
// source_path: inbox에서 온 경우 원본 경로(이동 후 삭제), None이면 storage에 직접 저장
async fn process_image(
    state: &AppState,
    file_data: Vec<u8>,
    file_name: String,
    source_path: Option<PathBuf>,
) -> Result<(usize, usize), Box<dyn std::error::Error + Send + Sync>> {
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
    for item in &analysis_results {
        let res = sqlx::query(
            r#"
            INSERT INTO costco_items
            (item_id, item_name, original_price, discount_amount, sale_price, discount_start, discount_end, price_tag_type, stock_status, image_url, uploaded_at)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
            "#,
        )
        .bind(&item.item_id)
        .bind(&item.item_name)
        .bind(item.original_price)
        .bind(item.discount_amount)
        .bind(item.sale_price)
        .bind(item.discount_start.as_ref().and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()))
        .bind(item.discount_end.as_ref().and_then(|s| chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d").ok()))
        .bind(&item.price_tag_type)
        .bind(&item.stock_status)
        .bind(processed_file_path.to_str())
        .bind(uploaded_at)
        .execute(&state.db)
        .await;

        if res.is_ok() {
            saved_count += 1;
        }
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

    Ok((analysis_results.len(), saved_count))
}

async fn analyze_with_gemini(api_key: &str, image_data: &[u8]) -> Result<Vec<AnalysisResult>, Box<dyn std::error::Error + Send + Sync>> {
    let base64_image = general_purpose::STANDARD.encode(image_data);
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/gemini-1.5-flash:generateContent?key={}",
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
    8. price_tag_type: 'Normal' (90), 'Double Discount' (70), 'Clearance' (00) based on last 2 digits of sale_price.
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

    let json_str = text.trim_matches(|c| c == '`' || c == '\n' || c == ' ');
    let json_str = if json_str.starts_with("json") { &json_str[4..] } else { json_str };

    let results: Vec<AnalysisResult> = serde_json::from_str(json_str)?;
    Ok(results)
}
