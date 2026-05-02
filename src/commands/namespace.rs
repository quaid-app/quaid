use anyhow::Result;
use clap::Subcommand;
use rusqlite::Connection;

use crate::core::namespace;

#[derive(Subcommand, Debug)]
pub enum NamespaceAction {
    /// Create namespace metadata
    Create {
        id: String,
        #[arg(long)]
        ttl: Option<f64>,
    },
    /// List namespace metadata
    List,
    /// Destroy a namespace and all pages assigned to it
    Destroy { id: String },
}

/// Run a namespace management command.
pub fn run(db: &Connection, action: NamespaceAction, json: bool) -> Result<()> {
    match action {
        NamespaceAction::Create { id, ttl } => create(db, &id, ttl, json),
        NamespaceAction::List => list(db, json),
        NamespaceAction::Destroy { id } => destroy(db, &id, json),
    }
}

fn create(db: &Connection, id: &str, ttl_hours: Option<f64>, json: bool) -> Result<()> {
    let namespace = namespace::create_namespace(db, id, ttl_hours)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&namespace)?);
    } else {
        println!("Created namespace {}", namespace.id);
    }
    Ok(())
}

fn list(db: &Connection, json: bool) -> Result<()> {
    let namespaces = namespace::list_namespaces(db)?;
    if json {
        println!("{}", serde_json::to_string_pretty(&namespaces)?);
    } else if namespaces.is_empty() {
        println!("No namespaces found.");
    } else {
        for namespace in namespaces {
            let ttl = namespace
                .ttl_hours
                .map(|hours| hours.to_string())
                .unwrap_or_else(|| "-".to_owned());
            println!("{}\t{}\t{}", namespace.id, ttl, namespace.created_at);
        }
    }
    Ok(())
}

fn destroy(db: &Connection, id: &str, json: bool) -> Result<()> {
    let deleted_pages = namespace::destroy_namespace(db, id)?;
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "status": "ok",
                "namespace": id,
                "deleted_pages": deleted_pages
            }))?
        );
    } else {
        println!("Destroyed namespace {id} ({deleted_pages} page(s) deleted)");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::db;

    fn open_test_db() -> Connection {
        db::open(":memory:").expect("open test db")
    }

    #[test]
    fn create_namespace_command_succeeds() {
        let conn = open_test_db();
        run(
            &conn,
            NamespaceAction::Create {
                id: "ns-a".to_string(),
                ttl: None,
            },
            false,
        )
        .expect("create ns");
        let namespaces = namespace::list_namespaces(&conn).expect("list");
        assert!(namespaces.iter().any(|n| n.id == "ns-a"));
    }

    #[test]
    fn create_namespace_command_with_json_output() {
        let conn = open_test_db();
        run(
            &conn,
            NamespaceAction::Create {
                id: "ns-json".to_string(),
                ttl: Some(24.0),
            },
            true,
        )
        .expect("create ns json");
    }

    #[test]
    fn list_namespaces_command_empty() {
        let conn = open_test_db();
        run(&conn, NamespaceAction::List, false).expect("list empty");
    }

    #[test]
    fn list_namespaces_command_with_entries() {
        let conn = open_test_db();
        run(
            &conn,
            NamespaceAction::Create {
                id: "list-me".to_string(),
                ttl: None,
            },
            false,
        )
        .expect("create");
        run(&conn, NamespaceAction::List, false).expect("list");
        run(&conn, NamespaceAction::List, true).expect("list json");
    }

    #[test]
    fn destroy_namespace_command_succeeds() {
        let conn = open_test_db();
        run(
            &conn,
            NamespaceAction::Create {
                id: "ns-del".to_string(),
                ttl: None,
            },
            false,
        )
        .expect("create");
        run(
            &conn,
            NamespaceAction::Destroy {
                id: "ns-del".to_string(),
            },
            false,
        )
        .expect("destroy");
        let namespaces = namespace::list_namespaces(&conn).expect("list");
        assert!(!namespaces.iter().any(|n| n.id == "ns-del"));
    }

    #[test]
    fn destroy_namespace_command_json_output() {
        let conn = open_test_db();
        run(
            &conn,
            NamespaceAction::Create {
                id: "ns-json-del".to_string(),
                ttl: None,
            },
            false,
        )
        .expect("create");
        run(
            &conn,
            NamespaceAction::Destroy {
                id: "ns-json-del".to_string(),
            },
            true,
        )
        .expect("destroy json");
    }
}
