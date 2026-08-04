//! Document extraction and code symbol engine. Port of the C# `Core/Extract.cs`,
//! `Core/DocxWriter.cs`, `Core/Ocr.cs`, `Core/TreeSitterSymbols.cs`,
//! `Core/CodeSearch.cs`, `Core/SymbolIndex.cs` and the `Mime` helper of
//! `Core/Support.cs`.
//!
//! The module is deliberately tool free: it exposes the engines that the
//! `tools::document` / `tools::search` families call, so the MCP surface and the
//! REST plane share one implementation.
//!
//! Coverage notes (see `extract` for the details): PDF, DOCX, PPTX, XLSX, HTML,
//! CSV, images (OCR) and plain text are supported. Audio and video are out of
//! scope and answer `ERR_NOT_SUPPORTED`, like the C#.

pub mod docx;
pub mod extract;
pub mod mime;
pub mod ocr;
pub mod symbols;

pub use docx::render_markdown_to_docx;
pub use extract::{ExtractResult, Extractor, companion_md_path, extract_text};
pub use mime::guess as guess_mime;
pub use ocr::{MultimodalOcrProvider, NullOcrProvider, OcrProvider, provider_from_config};
pub use symbols::{Definition, Reference, find_definitions, find_references, language_for};
