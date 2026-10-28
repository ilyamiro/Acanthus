/// Looks up translated strings for services that produce user-visible text.
pub trait Translator {
    /// The translation for `key`, or the key itself when missing.
    fn t(&self, key: &str) -> String;
    /// The translation for `key` with `{name}` placeholders replaced from `args`.
    fn t_args(&self, key: &str, args: &[(&str, &str)]) -> String;
}
