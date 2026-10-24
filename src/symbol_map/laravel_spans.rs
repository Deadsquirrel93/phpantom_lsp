//! The Laravel-specific pieces of string-key spans: config resources, and
//! the side tables for gate subjects and typed render receivers.

use super::{StringKeyKind, SubjectText, SymbolKind, SymbolSpan};

/// A Laravel resource whose short name is declared under a config subtree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum LaravelConfigResource {
    /// A guard under `auth.guards`.
    AuthGuard,
    /// A cache store under `cache.stores`.
    CacheStore,
    /// A logging channel under `logging.channels`.
    LogChannel,
    /// A filesystem disk under `filesystems.disks`.
    StorageDisk,
    /// A database connection under `database.connections`.
    DatabaseConnection,
    /// A queue connection under `queue.connections`.
    QueueConnection,
    /// A configured mailer under `mail.mailers`.
    Mailer,
    /// A broadcast connection under `broadcasting.connections`.
    BroadcastConnection,
}

impl LaravelConfigResource {
    /// The resource's unique bit in a zero-allocation family-presence set.
    pub(crate) const fn bit(self) -> u8 {
        match self {
            Self::AuthGuard => 1 << 0,
            Self::CacheStore => 1 << 1,
            Self::LogChannel => 1 << 2,
            Self::StorageDisk => 1 << 3,
            Self::DatabaseConnection => 1 << 4,
            Self::QueueConnection => 1 << 5,
            Self::Mailer => 1 << 6,
            Self::BroadcastConnection => 1 << 7,
        }
    }
}

/// The model a gate check names, recorded alongside its ability span.
///
/// Kept in a side table rather than inside [`SymbolKind::GateAbility`] so
/// that the (very hot) [`SymbolKind`] enum does not grow for every span in
/// every file: only the handful of gate calls in a project contribute an
/// entry, and only the unknown-ability diagnostic reads them.
#[derive(Debug, Clone)]
pub(crate) struct GateSubject {
    /// Start offset of the [`SymbolKind::GateAbility`] span this describes.
    pub ability_start: u32,
    /// Source text of the model expression — `Post` for `Post::class`,
    /// `$post` for a variable.
    pub subject_text: SubjectText,
    /// Whether the subject is a class name (`Post::class`) rather than a
    /// runtime value, which decides how it is resolved to a type.
    pub is_static: bool,
}

// ─── Render sites behind a typed receiver ───────────────────────────────────

/// The class a receiver has to be an instance of for the call it receives
/// to name a template.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ViewReceiverClass {
    /// Laravel's view factory, whose `make()` / `first()` / `exists()` /
    /// `renderWhen()` / `renderUnless()` / `renderEach()` all name one.
    Factory,
    /// A mailable, whose `text()` names the plain-text template the message
    /// is built from.
    Mailable,
    /// Anything a mail template is set on: a mailable, or the mail message a
    /// notification's `toMail()` returns.  The two share no base class, but
    /// `view()` and `markdown()` mean the same thing on both.
    MailTemplate,
}

impl ViewReceiverClass {
    /// The fully qualified names a receiver's type must be a subtype of one
    /// of.
    pub(crate) fn fqns(self) -> &'static [&'static str] {
        match self {
            // The concrete `Illuminate\View\Factory` implements the
            // contract, so the contract covers both spellings.
            Self::Factory => &["Illuminate\\Contracts\\View\\Factory"],
            Self::Mailable => &["Illuminate\\Mail\\Mailable"],
            Self::MailTemplate => &[
                "Illuminate\\Mail\\Mailable",
                "Illuminate\\Notifications\\Messages\\MailMessage",
            ],
        }
    }
}

/// A method call that names a template but whose viewness lives in its
/// receiver's *type* rather than its spelling: a constructor-injected
/// `Factory $views` behind `$this->views->make('page')`, or a mailable
/// held in a local behind `$mail->view('emails.shipped')`.
///
/// [`extract_symbol_map`] runs during `update_ast`, before the file's
/// classes are resolved, and cannot ask the forward walker what the
/// receiver is. It records the site instead, and
/// `Backend::typed_receiver_view_spans` settles it lazily.
#[derive(Debug, Clone)]
pub(crate) struct ViewReceiverSite {
    /// Byte offset of the first character of the view-name string's
    /// contents, matching what a [`SymbolKind::StringKey`] span
    /// records.
    pub start: u32,
    /// Byte offset one past the last character of those contents.
    pub end: u32,
    /// The view name in its canonical dotted spelling.
    pub key: String,
    /// Whether the call renders whichever of several candidates exists,
    /// so a name matching no template is the shape working as intended.
    pub is_optional: bool,
    /// What the receiver has to be for this to name a template at all.
    pub receiver: ViewReceiverClass,
}

impl ViewReceiverSite {
    /// The span this site contributes once its receiver is confirmed.
    pub(crate) fn to_span(&self) -> SymbolSpan {
        SymbolSpan {
            start: self.start,
            end: self.end,
            kind: SymbolKind::StringKey {
                key: self.key.clone(),
                kind: StringKeyKind::View,
                is_write: false,
                is_optional: self.is_optional,
            },
        }
    }
}
