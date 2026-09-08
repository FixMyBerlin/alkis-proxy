//! Bundesweiter ALKIS-Flurstücksdienst.
//!
//! Vereinheitlicht die WFS-Dienste der 16 Bundesländer hinter einer
//! OGC-API-Features-Schnittstelle mit einheitlichem GeoJSON-Output.

pub mod adapters;
pub mod api;
pub mod cache;
pub mod classification;
pub mod config;
pub mod crs;
pub mod model;
pub mod parse;
pub mod routing;
pub mod upstream;
