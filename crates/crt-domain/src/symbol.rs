/// What a symbol is. Structure sources translate their own vocabulary into
/// this one; the rest of the tool never sees source-specific names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Method,
    /// A struct, class, enum or type alias.
    Type,
    Interface,
    Module,
    /// A reference that calls something.
    Call,
    /// Anything the source reports that this tool has no use for yet.
    Other(String),
}

/// Where a symbol sits in the file. Bytes are half-open; lines are 1-based
/// and inclusive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start_byte: usize,
    pub end_byte: usize,
    pub start_line: usize,
    pub end_line: usize,
}

impl Span {
    /// True when `other` lies entirely inside this span.
    pub fn contains(&self, other: &Span) -> bool {
        self.start_byte <= other.start_byte && other.end_byte <= self.end_byte
    }

    /// Length in bytes.
    pub fn len(&self) -> usize {
        self.end_byte.saturating_sub(self.start_byte)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// True when the byte range is well-formed and inside `source`.
    pub fn fits(&self, source: &[u8]) -> bool {
        self.start_byte <= self.end_byte && self.end_byte <= source.len()
    }
}

/// A definition or a reference found in one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub kind: SymbolKind,
    pub is_definition: bool,
    /// The whole definition or reference.
    pub span: Span,
    /// The line on which the name itself appears. For a multi-line call
    /// chain this is the line of the callee, which is where a reader looks.
    pub name_line: usize,
    /// The type this method belongs to, when the source states it outright
    /// (a Go receiver, a Rust `impl` block). `None` means "not stated";
    /// the enclosing type may still be found by containment.
    pub owner: Option<String>,
    pub docs: Option<String>,
}

impl Symbol {
    pub fn is_function_like(&self) -> bool {
        self.is_definition && matches!(self.kind, SymbolKind::Function | SymbolKind::Method)
    }

    /// Definitions that can own methods by containment: classes and
    /// interfaces. Modules are containers but not types, so a free function
    /// inside a module has no enclosing type.
    pub fn is_type_like(&self) -> bool {
        self.is_definition && matches!(self.kind, SymbolKind::Type | SymbolKind::Interface)
    }

    pub fn is_call(&self) -> bool {
        !self.is_definition && self.kind == SymbolKind::Call
    }
}
