//! B09 P8a and P8b — money and time as table columns on SQLite and,
//! when `CORVID_TEST_POSTGRES_URL` is set, on Postgres. Each type is
//! stored in a column the database compares exactly as Corvid does:
//! decimals and durations by value (so `10 == 10.0` and `9.50` sorts
//! before `10.0`), instants in time order across fraction lengths, and
//! zoned times, zones, currencies and locales by equality. Every value
//! reads back exactly, scale and nanoseconds included.

use corvid_driver::{compile_to_ir, run_ir_with_runtime};
use corvid_runtime::Runtime;
use corvid_vm::Value;

#[path = "support/table_engines.rs"]
mod table_engines;
use table_engines::TestDatabase;

fn program(settings: &str) -> String {
    format!(
        r#"
database ledger:
{settings}

table Invoice:
    key number: Int
    total: Decimal
    issued: Instant
    terms: Duration
    currency: Currency
    office: TimeZone
    locale: Locale
    signed: Option<ZonedTime>

fn at(text: String) -> Instant:
    return match text.parse_instant():
        Ok(instant) -> instant
        Err(_) -> Instant.from_epoch_milliseconds(0)

fn span(text: String) -> Duration:
    return match text.parse_duration():
        Ok(duration) -> duration
        Err(_) -> at("1970-01-01T00:00:00Z") - at("1970-01-01T00:00:00Z")

agent add(number: Int, total: Decimal, issued: String, terms: String, euro: Bool) -> String:
    eur: Currency = "EUR"
    gbp: Currency = "GBP"
    berlin: TimeZone = "Europe/Berlin"
    london: TimeZone = "Europe/London"
    british: Locale = "en-GB"
    currency = gbp
    office = london
    signed: Option<ZonedTime> = None
    if euro:
        currency = eur
        office = berlin
        signed = Some(at(issued).in_zone(berlin))
    return match Invoice.insert(Invoice(number, total, at(issued), span(terms), currency, office, british, signed)):
        Ok(row) -> "+" + row.number.to_string()
        Err(_) -> "taken"

fn numbers(rows: List<Invoice>) -> String:
    out = ""
    for row in rows:
        out = out + row.number.to_string() + ","
    return out

agent large() -> String:
    return numbers(Invoice.where(fn (i) -> i.total > 100.00).order_by(fn (i) -> i.total))

agent ten() -> String:
    return numbers(Invoice.where(fn (i) -> i.total == 10))

agent by_total() -> String:
    return numbers(Invoice.all().order_by(fn (i) -> i.total))

agent since(text: String) -> String:
    start = at(text)
    return numbers(Invoice.where(fn (i) -> i.issued >= start).order_by(fn (i) -> i.issued))

agent longest_terms() -> String:
    return numbers(Invoice.all().order_by(fn (i) -> i.terms).descending())

agent in_euros() -> String:
    eur: Currency = "EUR"
    return numbers(Invoice.where(fn (i) -> i.currency == eur))

agent signed_at(text: String) -> String:
    berlin: TimeZone = "Europe/Berlin"
    moment = Some(at(text).in_zone(berlin))
    return numbers(Invoice.where(fn (i) -> i.signed == moment))

agent read_back(number: Int) -> String:
    return match Invoice.get(number):
        Some(row) -> row.total.to_string() + " " + row.issued.to_string() + " " + row.terms.to_string() + " " + row.currency.to_string() + " " + row.office.to_string() + " " + row.locale.to_string()
        None -> "none"

agent main() -> String:
    added = add(1, 10.0, "2026-07-11T08:30:00.5Z", "PT2H", true) + add(2, 9.50, "2026-07-11T08:30:00.000000001Z", "PT59M", false) + add(3, 1000.00, "2026-07-11T08:30:01Z", "-PT1S", true) + add(4, 100.5, "1969-12-31T23:59:59.999Z", "PT0.5S", false) + add(5, -3, "2026-03-29T03:30:00+02:00", "PT1H30M", true)
    return added + " | " + large() + " | " + ten() + " | " + by_total() + " | " + since("2026-07-11T08:30:00Z") + " | " + longest_terms() + " | " + in_euros() + " | " + signed_at("2026-03-29T01:30:00Z") + " | " + read_back(2) + " | " + read_back(1)
"#
    )
}

const EXPECTED: &str = "+1+2+3+4+5 | 4,3, | 1, | 5,2,1,4,3, | 2,1,3, | 1,5,2,4,3, | 1,3,5, | 5, | 9.50 2026-07-11T08:30:00.000000001Z PT59M GBP Europe/London en-GB | 10.0 2026-07-11T08:30:00.500Z PT2H EUR Europe/Berlin en-GB";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn money_and_time_columns_compare_and_read_back_exactly() {
    for database in TestDatabase::every_engine().await {
        let source = program(&database.settings());
        database.create_tables(&source).await;
        let ir = compile_to_ir(&source).unwrap_or_else(|errors| panic!("must compile: {errors:#?}"));
        let runtime = Runtime::builder().build();
        let out = run_ir_with_runtime(&ir, Some("main"), vec![], &runtime).await.expect("program runs");
        assert_eq!(out, Value::String(EXPECTED.into()), "on {}", database.label());
        database.finish().await;
    }
}

#[test]
fn each_type_declares_its_own_column_and_decimals_their_collation() {
    let source = program("    engine: sqlite\n    location: \"ledger.sqlite\"");
    let schemas = corvid_driver::table_schemas(&source, std::path::Path::new("ledger.cor")).expect("compiles");
    let sql = corvid_runtime::tables::create_table_sql(&schemas[0]);
    for column in [
        "\"total\" DECIMAL TEXT COLLATE corvid_decimal NOT NULL",
        "\"issued\" INSTANT TEXT NOT NULL",
        "\"terms\" DURATION TEXT COLLATE corvid_duration NOT NULL",
        "\"currency\" CURRENCY TEXT NOT NULL",
        "\"office\" TIME ZONE TEXT NOT NULL",
        "\"locale\" LOCALE TEXT NOT NULL",
        "\"signed\" ZONED TIME TEXT,",
    ] {
        assert!(sql.contains(column), "missing {column} in {sql}");
    }
}

fn errors(source: &str) -> String {
    match compile_to_ir(source) {
        Ok(_) => panic!("must be refused:\n{source}"),
        Err(errors) => format!("{errors:#?}"),
    }
}

const TABLE: &str = r#"
database ledger:
    engine: sqlite
    location: "ledger.sqlite"

table Invoice:
    key number: Int
    signed: ZonedTime
    office: TimeZone
    notes: List<String>
"#;

#[test]
fn ordering_by_a_type_without_an_order_or_by_json_is_refused() {
    let zoned = errors(&format!("{TABLE}\nagent q() -> List<Invoice>:\n    return Invoice.all().order_by(fn (i) -> i.signed)\n"));
    assert!(zoned.contains("`order_by` reads column `signed`, a `ZonedTime`, which has no order"), "{zoned}");
    let zone = errors(&format!("{TABLE}\nagent q() -> List<Invoice>:\n    return Invoice.all().order_by(fn (i) -> i.office)\n"));
    assert!(zone.contains("a `TimeZone`, which has no order"), "{zone}");
    let json = errors(&format!("{TABLE}\nagent q() -> List<Invoice>:\n    return Invoice.all().order_by(fn (i) -> i.notes)\n"));
    assert!(json.contains("column `notes` is stored as JSON, so the database cannot order it"), "{json}");
}

#[test]
fn postgres_declares_each_type_with_a_domain_created_first() {
    let source = program("    engine: postgres\n    location: env(\"LEDGER_URL\")\n    tls: required\n    connections: 4");
    let schemas = corvid_driver::table_schemas(&source, std::path::Path::new("ledger.cor")).expect("compiles");
    let sql = corvid_runtime::tables::create_table_sql_on(&schemas[0], schemas[0].engine);
    for (domain, base) in [
        ("corvid_decimal", "numeric"),
        ("corvid_duration", "numeric(39, 0)"),
        ("corvid_instant", "text COLLATE \"C\" CHECK"),
        ("corvid_currency", "text COLLATE \"C\""),
        ("corvid_zoned_time", "text COLLATE \"C\""),
    ] {
        let create = format!("CREATE DOMAIN {domain} AS {base}");
        assert!(sql.contains(&create), "missing {create} in {sql}");
        assert_eq!(sql.matches(&format!("CREATE DOMAIN {domain} ")).count(), 1, "{sql}");
        assert!(sql.find(&create) < sql.find("CREATE TABLE"), "{sql}");
    }
    assert!(sql.contains("\"total\" corvid_decimal NOT NULL"), "{sql}");
    assert!(sql.contains("\"signed\" corvid_zoned_time,"), "{sql}");
}
