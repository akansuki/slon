-- Demo schema for the README screenshots (see screenshots.mjs).
-- Load it into the throwaway test cluster:
--   eval "$(../scripts/test-db.sh start)"
--   PGPASSWORD=$SLON_TEST_PASSWORD psql -h $SLON_TEST_HOST -p $SLON_TEST_PORT -U $SLON_TEST_USER -d $SLON_TEST_DB -f demo.sql

DROP SCHEMA IF EXISTS shop CASCADE;
CREATE SCHEMA shop;
SET search_path = shop;

CREATE TABLE customers (
  id          bigserial PRIMARY KEY,
  email       text NOT NULL UNIQUE,
  full_name   text NOT NULL,
  country     char(2) NOT NULL,
  created_at  timestamp NOT NULL DEFAULT now(),
  is_active   boolean NOT NULL DEFAULT true
);
COMMENT ON TABLE customers IS 'People who have placed at least one order';

CREATE TABLE products (
  id          bigserial PRIMARY KEY,
  sku         text NOT NULL UNIQUE,
  name        text NOT NULL,
  price       numeric(10,2) NOT NULL CHECK (price >= 0),
  in_stock    integer NOT NULL DEFAULT 0
);

CREATE TABLE orders (
  id           bigserial PRIMARY KEY,
  customer_id  bigint NOT NULL REFERENCES customers(id),
  placed_at    timestamp NOT NULL DEFAULT now(),
  status       text NOT NULL DEFAULT 'pending',
  total        numeric(12,2) NOT NULL,
  shipped_at   timestamp,
  note         text
);
CREATE INDEX orders_customer_idx ON orders (customer_id);
CREATE INDEX orders_placed_idx ON orders (placed_at DESC);
CREATE INDEX orders_status_idx ON orders (status) WHERE status <> 'shipped';

CREATE TABLE order_lines (
  order_id    bigint NOT NULL REFERENCES orders(id) ON DELETE CASCADE,
  line_no     integer NOT NULL,
  product_id  bigint NOT NULL REFERENCES products(id),
  quantity    integer NOT NULL CHECK (quantity > 0),
  unit_price  numeric(10,2) NOT NULL,
  PRIMARY KEY (order_id, line_no)
);

CREATE VIEW recent_orders AS
  SELECT o.id, c.full_name, o.placed_at, o.status, o.total
  FROM orders o JOIN customers c ON c.id = o.customer_id
  ORDER BY o.placed_at DESC;

CREATE FUNCTION touch_order() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
  RAISE NOTICE 'order % changed to %', NEW.id, NEW.status;
  RETURN NEW;
END $$;
CREATE TRIGGER orders_audit AFTER UPDATE ON orders FOR EACH ROW EXECUTE FUNCTION touch_order();

INSERT INTO customers (email, full_name, country, created_at)
SELECT format('user%s@example.com', g),
       (ARRAY['Ana Petrović','Jonas Weber','Marie Dubois','Luca Rossi','Emma Johansson','Pieter de Vries','Sofia Moreno','Tomáš Novák'])[1 + (g % 8)],
       (ARRAY['RS','DE','FR','IT','SE','NL','ES','CZ'])[1 + (g % 8)],
       now() - (g || ' hours')::interval
FROM generate_series(1, 400) g;

INSERT INTO products (sku, name, price, in_stock)
SELECT format('SKU-%04s', g),
       (ARRAY['Walnut desk lamp','Linen cushion cover','Ceramic mug','Oak cutting board','Wool throw blanket','Glass carafe','Cast iron pan','Cotton tea towel'])[1 + (g % 8)],
       round((12 + random() * 180)::numeric, 2),
       (random() * 250)::int
FROM generate_series(1, 120) g;

INSERT INTO orders (customer_id, placed_at, status, total, shipped_at, note)
SELECT 1 + (g % 400),
       now() - (g || ' minutes')::interval,
       (ARRAY['shipped','shipped','shipped','pending','cancelled'])[1 + (g % 5)],
       round((20 + random() * 400)::numeric, 2),
       CASE WHEN g % 5 < 3 THEN now() - (g || ' minutes')::interval + interval '2 days' END,
       CASE WHEN g % 17 = 0 THEN 'Gift wrap requested' END
FROM generate_series(1, 5000) g;

INSERT INTO order_lines (order_id, line_no, product_id, quantity, unit_price)
SELECT o.id, l, 1 + ((o.id * l) % 120), 1 + ((o.id + l) % 4),
       round((12 + random() * 180)::numeric, 2)
FROM orders o, generate_series(1, 3) l;

ANALYZE;
