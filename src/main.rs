use axum::{
    extract::{DefaultBodyLimit, Multipart, State},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgPoolOptions;
use sqlx::{Pool, Postgres};
use std::sync::Arc;
use dotenvy::dotenv;
use tokio::fs;
use std::path::Path;
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
}

#[tokio::main]
async fn main() {
    dotenv().ok();
    tracing_subscriber::fmt::init();

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set");
    let gemini_api_key = std::env::var("GEMINI_API_KEY").expect("GEMINI_API_KEY must be set");
    let storage_path = std::env::var("STORAGE_PATH").unwrap_or_else(|_| "./storage".to_string());
    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());

    // 스토리지 디렉토리 생성
    if !Path::new(&storage_path).exists() {
        fs::create_dir_all(&storage_path).await.expect("Failed to create storage directory");
    }

    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect(&database_url)
        .await
        .expect("Failed to connect to Postgres");

    let shared_state = Arc::new(AppState {
        db: pool,
        gemini_api_key,
        storage_path,
    });

    let app = Router::new()
        .route("/health", get(|| async { "OK" }))
        .route("/upload", post(upload_handler))
        .layer(DefaultBodyLimit::max(20 * 1024 * 1024)) // 20MB
        .with_state(shared_state);

    let addr = format!("0.0.0.0:{}", port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tracing::info!("listening on {}", listener.local_addr().unwrap());
    axum::serve(listener, app).await.unwrap();
}

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
            // 타임스탬프를 붙여 중복 방지
            file_name = format!("{}_{}", chrono::Utc::now().timestamp(), file_name);
            file_data = field.bytes().await.unwrap().to_vec();
        }
    }

    if file_data.is_empty() {
        return Json(json!({ "error": "No image uploaded" }));
    }

    // 1. 이미지 저장
    let file_path = Path::new(&state.storage_path).join(&file_name);
    if let Err(e) = fs::write(&file_path, &file_data).await {
        return Json(json!({ "error": format!("Failed to save image: {}", e) }));
    }

    // 2. Gemini API 호출
    let analysis_results = match analyze_with_gemini(&state.gemini_api_key, &file_data).await {
        Ok(results) => results,
        Err(e) => return Json(json!({ "error": format!("AI analysis failed: {}", e) })),
    };

    // 3. DB 저장
    let mut saved_count = 0;
    for item in &analysis_results {
        let res = sqlx::query(
            r#"
            INSERT INTO costco_items
            (item_id, item_name, original_price, discount_amount, sale_price, discount_start, discount_end, price_tag_type, stock_status, image_url)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
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
        .bind(file_path.to_str())
        .execute(&state.db)
        .await;

        if res.is_ok() {
            saved_count += 1;
        }
    }

    Json(json!({ 
        "status": "success", 
        "detected_count": analysis_results.len(),
        "saved_count": saved_count,
        "items": analysis_results 
    }))
}

async fn analyze_with_gemini(api_key: &str, image_data: &[u8]) -> Result<Vec<AnalysisResult>, Box<dyn std::error::Error>> {
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
