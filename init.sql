-- 코스트코 상품 정보 테이블 생성
CREATE TABLE IF NOT EXISTS costco_items (
    idx SERIAL PRIMARY KEY,
    item_id VARCHAR(20) NOT NULL,
    item_name VARCHAR(255) NOT NULL,
    original_price INT,
    discount_amount INT,
    sale_price INT NOT NULL,
    discount_start DATE,
    discount_end DATE,
    price_tag_type VARCHAR(50), -- Normal, DoubleDiscount, Clearance
    stock_status VARCHAR(50),    -- InStock, Discontinued
    image_url TEXT,
    uploaded_at TIMESTAMPTZ,
    reg_date TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- 인덱스 생성
CREATE INDEX IF NOT EXISTS idx_item_id ON costco_items(item_id);
CREATE INDEX IF NOT EXISTS idx_discount_end ON costco_items(discount_end);
CREATE INDEX IF NOT EXISTS idx_uploaded_at ON costco_items(uploaded_at);
CREATE INDEX IF NOT EXISTS idx_item_id_idx ON costco_items(item_id, idx DESC);
CREATE INDEX IF NOT EXISTS idx_discount_partial ON costco_items(item_id, idx DESC) WHERE discount_amount IS NOT NULL;

-- 텍스트 검색 속도 향상 (trigram)
CREATE EXTENSION IF NOT EXISTS pg_trgm;
CREATE INDEX IF NOT EXISTS idx_item_name_trgm ON costco_items USING GIN (item_name gin_trgm_ops);
CREATE INDEX IF NOT EXISTS idx_category_trgm ON costco_items USING GIN (category gin_trgm_ops);
