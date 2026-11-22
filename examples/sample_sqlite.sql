-- Creates examples/sample.sqlite, a SQLite database that examples/sample_db.dvz reads.
-- The file is not checked in. Create it with the sqlite3 command line tool:
--
--   sqlite3 examples/sample.sqlite < examples/sample_sqlite.sql
--
-- Then point the connection reference "sample_db" at it (SQLite connections are file paths):
--
--   set DAVILIZE_CONN_SAMPLE_DB=examples\sample.sqlite
--   davilize run-sql examples\sample_db.dvz "SELECT product, sum(units) AS units FROM sample_db.orders GROUP BY product ORDER BY units DESC"

DROP TABLE IF EXISTS orders;
DROP TABLE IF EXISTS customers;

CREATE TABLE orders (
    order_id INTEGER PRIMARY KEY,
    order_date TEXT NOT NULL,
    region TEXT NOT NULL,
    product TEXT NOT NULL,
    units INTEGER NOT NULL,
    unit_price REAL NOT NULL,
    customer TEXT NOT NULL
);

INSERT INTO orders (order_id, order_date, region, product, units, unit_price, customer) VALUES
    (1001, '2026-01-03', 'North', 'Widget', 4, 12.50, 'acme'),
    (1002, '2026-01-05', 'South', 'Gadget', 2, 30.00, 'globex'),
    (1003, '2026-01-07', 'West', 'Widget', 10, 12.50, 'initech'),
    (1004, '2026-01-09', 'North', 'Gizmo', 1, 99.00, 'acme'),
    (1005, '2026-01-12', 'East', 'Gadget', 3, 30.00, 'umbrella'),
    (1006, '2026-01-14', 'South', 'Widget', 6, 12.50, 'globex'),
    (1007, '2026-01-18', 'West', 'Gizmo', 2, 99.00, 'hooli'),
    (1008, '2026-01-21', 'North', 'Gadget', 5, 30.00, 'initech');

CREATE TABLE customers (
    customer TEXT PRIMARY KEY,
    segment TEXT NOT NULL
);

INSERT INTO customers (customer, segment) VALUES
    ('acme', 'enterprise'),
    ('globex', 'smb'),
    ('initech', 'enterprise'),
    ('umbrella', 'smb'),
    ('hooli', 'enterprise');
