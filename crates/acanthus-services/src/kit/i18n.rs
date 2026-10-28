pub trait Translator {
    fn t(&self, key: &str) -> String;
    fn t_args(&self, key: &str, args: &[(&str, &str)]) -> String;
}
