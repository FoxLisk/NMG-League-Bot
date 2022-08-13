use diesel_enum_derive::DieselEnum;

#[derive(Debug, DieselEnum)]
enum S {
    A,
    B,
}

fn main() {
    let s: String = S::A.into();
    println!("{}", s);
    println!("{:?}", S::try_from("B".to_string()));
    println!("{:?}", S::try_from("C".to_string()));
}