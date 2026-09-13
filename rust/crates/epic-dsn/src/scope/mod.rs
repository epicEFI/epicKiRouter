//! The scope readers (M1b Tasks 5-9): one module per Java parser class
//! that reads a top-level DSN scope. Task 5 lands the `(structure ...)`
//! reader ([`structure`]) and the `Rule.java` readers ([`rule`]) it
//! drives. Task 7 lands [`placement`] (`Placement.java` +
//! `Component.java`), [`library`] (`Library.java` + `Package.java`), and
//! the `PartLibrary.java` half of [`library`].

pub mod library;
pub mod network;
pub mod placement;
pub mod plane;
pub mod rule;
pub mod structure;
