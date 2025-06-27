use std::usize;

const HTTPS: &'static str = "https://";
const MIDDLE: &'static str = "/resto/api";

pub fn default(server: &String, path: &[&str]) -> String {
    let mut string = String::with_capacity(
        HTTPS.len()
            + server.len()
            + MIDDLE.len()
            + path.len()
            + path.iter().map(|element| element.len()).sum::<usize>(),
    );

    string.push_str(HTTPS);

    string.push_str(server);

    string.push_str(MIDDLE);

    for element in path {
        string.push('/');
        string.push_str(&element);
    }

    println!("{string}");

    string
}
