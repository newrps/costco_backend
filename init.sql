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
    reg_date TIMESTAMP DEFAULT CURRENT_TIMESTAMP
);

-- 인덱스 생성
CREATE INDEX IF NOT EXISTS idx_item_id ON costco_items(item_id);
CREATE INDEX IF NOT EXISTS idx_discount_end ON costco_items(discount_end);
