//! Indexes derived from a table's queries (B09 P9).
//!
//! A table is private to its module, so the compiler sees every query it
//! will ever get. Each query asks the database to find rows by some
//! columns and perhaps order them by another; the index that serves it is
//! those equality columns, in declaration order, then the ordered or
//! compared column. The migration planner creates the indexes the queries
//! need and drops the ones Corvid made that no query needs any more, so
//! indexes follow the code instead of a list kept beside it.

use super::{quote, TableSchema};

/// What one query asks of the database, for choosing its index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryShape {
    /// The agent that runs the query, named in the migration.
    pub agent: String,
    /// Columns the condition requires to equal a value, joined by `and`
    /// at the top level of the condition.
    pub equal: Vec<String>,
    /// A column the condition compares with `<`, `<=`, `>` or `>=` at the
    /// top level of the condition.
    pub range: Option<String>,
    /// The column `order_by` reads.
    pub order: Option<String>,
}

/// An index a table's queries need, or one of its `unique` rules.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableIndex {
    pub name: String,
    pub columns: Vec<String>,
    /// The agents whose queries it serves.
    pub agents: Vec<String>,
    /// Whether it enforces a `unique` rule.
    pub unique: bool,
}

/// The longest identifier Postgres keeps; a longer one is cut short, and
/// a cut name would never match the planned one again.
const MAX_NAME: usize = 63;

/// The indexes `table` needs: one unique index per `unique` rule, then
/// one for each query. `columns` is the table's columns in declaration
/// order. A query the key or a unique index already serves, in whole or as
/// a leading part, needs none, and an index whose columns lead another's
/// is served by that one.
pub fn derive_indexes(
    table: &str,
    keys: &[String],
    unique: &[Vec<String>],
    columns: &[String],
    shapes: &[QueryShape],
) -> Result<Vec<TableIndex>, String> {
    let position = |column: &String| columns.iter().position(|c| c == column).unwrap_or(usize::MAX);
    let mut wanted: Vec<(Vec<String>, Vec<String>)> = Vec::new();
    for shape in shapes {
        let mut index = shape.equal.clone();
        index.sort_by_key(position);
        index.dedup();
        if let Some(last) = shape.order.as_ref().or(shape.range.as_ref()) {
            if !index.contains(last) {
                index.push(last.clone());
            }
        }
        if index.is_empty() || leads(&index, keys) || unique.iter().any(|rule| leads(&index, rule)) {
            continue;
        }
        match wanted.iter_mut().find(|(columns, _)| *columns == index) {
            Some((_, agents)) => agents.push(shape.agent.clone()),
            None => wanted.push((index, vec![shape.agent.clone()])),
        }
    }
    // An index whose columns lead a longer one is served by the longer one.
    let mut kept: Vec<(Vec<String>, Vec<String>)> = Vec::new();
    for (columns, agents) in &wanted {
        let longer = wanted.iter().find(|(other, _)| other.len() > columns.len() && leads(columns, other));
        match longer {
            Some((other, _)) => {
                let into = kept.iter_mut().find(|(c, _)| c == other);
                match into {
                    Some((_, served)) => served.extend(agents.iter().cloned()),
                    None => kept.push((other.clone(), agents.clone())),
                }
            }
            None => match kept.iter_mut().find(|(c, _)| c == columns) {
                Some((_, served)) => served.extend(agents.iter().cloned()),
                None => kept.push((columns.clone(), agents.clone())),
            },
        }
    }
    let mut indexes = kept
        .into_iter()
        .map(|(columns, mut agents)| {
            agents.sort();
            agents.dedup();
            Ok(TableIndex { name: index_name(table, "by", &columns)?, columns, agents, unique: false })
        })
        .collect::<Result<Vec<_>, String>>()?;
    for rule in unique {
        indexes.push(TableIndex { name: index_name(table, "unique", rule)?, columns: rule.clone(), agents: Vec::new(), unique: true });
    }
    indexes.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(indexes)
}

/// Whether `prefix` is `of`'s leading columns.
fn leads(prefix: &[String], of: &[String]) -> bool {
    prefix.len() <= of.len() && of[..prefix.len()] == *prefix
}

/// The start of every lookup index name Corvid makes for `table`. The
/// planner drops only indexes whose names start with it or with
/// [`unique_prefix`].
pub fn managed_prefix(table: &str) -> String {
    format!("{table}_by_")
}

/// The start of every unique index name Corvid makes for `table`.
pub fn unique_prefix(table: &str) -> String {
    format!("{table}_unique_")
}

/// `Table_by_owner_job` or `Table_unique_email`, or, past Postgres's
/// 63-byte limit, the prefix and a stable digest of the columns.
fn index_name(table: &str, kind: &str, columns: &[String]) -> Result<String, String> {
    let prefix = format!("{table}_{kind}_");
    let readable = format!("{prefix}{}", columns.join("_"));
    if readable.len() <= MAX_NAME {
        return Ok(readable);
    }
    let digest = columns.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, column| {
        column.bytes().chain(std::iter::once(0)).fold(hash, |h, b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
    });
    let short = format!("{prefix}{digest:016x}");
    if short.len() <= MAX_NAME {
        Ok(short)
    } else {
        Err(format!(
            "table `{table}`: its name is too long to name an index within Postgres's 63 bytes; give the table a shorter name"
        ))
    }
}

/// The statements that bring `stored` (the names of the table's indexes,
/// as the database has them) to the indexes `table` needs, each with the
/// comment the migration shows: indexes Corvid made that no query needs
/// are dropped, and missing ones are created.
pub fn plan_index_changes(table: &TableSchema, stored: &[String]) -> Vec<(String, String)> {
    let (lookup, unique) = (managed_prefix(&table.name), unique_prefix(&table.name));
    let mut statements = Vec::new();
    for name in stored.iter().filter(|name| name.starts_with(&lookup) || name.starts_with(&unique)) {
        if !table.indexes.iter().any(|index| &index.name == name) {
            statements.push((
                format!("`{}`: drop index `{name}`, which no query or `unique` line needs any more", table.name),
                format!("DROP INDEX IF EXISTS {};", quote(name)),
            ));
        }
    }
    for index in table.indexes.iter().filter(|index| !stored.contains(&index.name)) {
        let columns = index.columns.iter().map(|c| quote(c)).collect::<Vec<_>>().join(", ");
        let (comment, create) = if index.unique {
            (format!("`{}`: no two rows share ({})", table.name, index.columns.join(", ")), "CREATE UNIQUE INDEX")
        } else {
            (format!("`{}`: index ({}) for the queries in {}", table.name, index.columns.join(", "), agents_list(&index.agents)), "CREATE INDEX")
        };
        statements.push((comment, format!("{create} IF NOT EXISTS {} ON {} ({columns});", quote(&index.name), quote(&table.name))));
    }
    statements
}

fn agents_list(agents: &[String]) -> String {
    agents.iter().map(|agent| format!("`{agent}`")).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    fn shape(agent: &str, equal: &[&str], range: Option<&str>, order: Option<&str>) -> QueryShape {
        QueryShape { agent: agent.into(), equal: strings(equal), range: range.map(Into::into), order: order.map(Into::into) }
    }

    const COLUMNS: &[&str] = &["job", "owner", "paused", "budget", "revision"];

    fn derived(shapes: &[QueryShape]) -> Vec<(String, Vec<String>, Vec<String>)> {
        derive_indexes("Job", &strings(&["job"]), &[], &strings(COLUMNS), shapes)
            .unwrap()
            .into_iter()
            .map(|i| (i.name, i.columns, i.agents))
            .collect()
    }

    #[test]
    fn equality_columns_in_declaration_order_then_the_ordered_column() {
        let indexes = derived(&[shape("latest", &["paused", "owner"], None, Some("budget"))]);
        assert_eq!(indexes, [("Job_by_owner_paused_budget".into(), strings(&["owner", "paused", "budget"]), strings(&["latest"]))]);
    }

    #[test]
    fn the_key_serves_its_own_lookups() {
        assert!(derived(&[shape("get", &["job"], None, None), shape("all", &[], None, Some("job"))]).is_empty());
        assert!(derived(&[shape("all", &[], None, None)]).is_empty());
    }

    #[test]
    fn a_leading_index_is_served_by_the_longer_one() {
        let indexes = derived(&[shape("mine", &["owner"], None, None), shape("latest", &["owner"], None, Some("job"))]);
        assert_eq!(indexes, [("Job_by_owner_job".into(), strings(&["owner", "job"]), strings(&["latest", "mine"]))]);
    }

    #[test]
    fn a_range_column_comes_last_when_nothing_is_ordered() {
        let indexes = derived(&[shape("cheap", &["owner"], Some("budget"), None)]);
        assert_eq!(indexes[0].1, strings(&["owner", "budget"]));
    }

    #[test]
    fn a_unique_rule_is_a_unique_index_that_also_serves_lookups() {
        let rule = vec![strings(&["owner", "budget"])];
        let indexes = derive_indexes("Job", &strings(&["job"]), &rule, &strings(COLUMNS), &[shape("mine", &["owner"], None, None)]).unwrap();
        assert_eq!(indexes.len(), 1, "{indexes:?}");
        assert_eq!((indexes[0].name.as_str(), indexes[0].unique), ("Job_unique_owner_budget", true));
    }

    #[test]
    fn long_names_stay_within_postgres_limits_and_are_stable() {
        let long = "a_column_with_a_name_long_enough_to_push_past_the_limit";
        let columns = strings(&["job", long, "other_long_column_name"]);
        let shapes = [shape("q", &[long, "other_long_column_name"], None, None)];
        let first = derive_indexes("Job", &strings(&["job"]), &[], &columns, &shapes).unwrap();
        let again = derive_indexes("Job", &strings(&["job"]), &[], &columns, &shapes).unwrap();
        assert!(first[0].name.len() <= MAX_NAME && first[0].name.starts_with("Job_by_"), "{}", first[0].name);
        assert_eq!(first, again);
    }
}
