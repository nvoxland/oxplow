//! The person's left-nav layout (`panel_layout`, P6.G1): each panel's
//! position, and whether it's hidden or collapsed. Panels are named by id
//! (`core:work`, `ext:<extension>/<panel>`); one the table doesn't name
//! shows expanded after the ones it does. Local to this project's
//! database, never the repo.

use oxplow_domain::DomainError;
use serde::{Deserialize, Serialize};

use crate::database::map_sql_err;
use crate::Database;

/// One panel's place in the layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct PanelPlacement {
    pub panel: String,
    pub hidden: bool,
    pub collapsed: bool,
}

pub struct SqlitePanelLayoutStore {
    db: Database,
}

impl SqlitePanelLayoutStore {
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// The layout, in order.
    pub async fn get(&self) -> Result<Vec<PanelPlacement>, DomainError> {
        self.db
            .read(|c| {
                let mut stmt = c
                    .prepare("SELECT panel, hidden, collapsed FROM panel_layout ORDER BY position")
                    .map_err(map_sql_err)?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok(PanelPlacement {
                            panel: r.get(0)?,
                            hidden: r.get::<_, i64>(1)? == 1,
                            collapsed: r.get::<_, i64>(2)? == 1,
                        })
                    })
                    .map_err(map_sql_err)?
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .map_err(map_sql_err)?;
                Ok(rows)
            })
            .await
    }

    /// Replace the layout with `layout`, in its order.
    pub async fn set(&self, layout: Vec<PanelPlacement>) -> Result<(), DomainError> {
        self.db
            .transaction(move |tx| {
                tx.execute("DELETE FROM panel_layout", []).map_err(map_sql_err)?;
                for (i, p) in layout.iter().enumerate() {
                    tx.execute(
                        "INSERT INTO panel_layout (panel, position, hidden, collapsed) VALUES (?1, ?2, ?3, ?4)",
                        rusqlite::params![p.panel, i as i64, p.hidden as i64, p.collapsed as i64],
                    )
                    .map_err(map_sql_err)?;
                }
                Ok(())
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// P6.G1: the layout persists in order, and a new set replaces it.
    #[tokio::test]
    async fn the_layout_persists_in_order() {
        let store = SqlitePanelLayoutStore::new(Database::in_memory());
        assert!(store.get().await.unwrap().is_empty());
        let place = |panel: &str, hidden, collapsed| PanelPlacement {
            panel: panel.into(),
            hidden,
            collapsed,
        };
        let layout = vec![
            place("core:comments", false, true),
            place("core:work", false, false),
            place("ext:gh/prs", true, false),
        ];
        store.set(layout.clone()).await.unwrap();
        assert_eq!(store.get().await.unwrap(), layout);
        store
            .set(vec![place("core:work", false, false)])
            .await
            .unwrap();
        assert_eq!(
            store.get().await.unwrap(),
            vec![place("core:work", false, false)]
        );
    }
}
