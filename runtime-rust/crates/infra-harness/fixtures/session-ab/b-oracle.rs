mod candidate { include!("/input/source.rs"); }
fn main() {
    for value in ["0s","42s","2m","1h","001m","18446744073709551615s","18446744073709551615m","18446744073709551616s","", "s","-1s","+1s","1.5m","1d","1S"," 1s","1s ","١s"] {
        match candidate::parse_duration(value) {
            Ok(seconds) => println!("D:{seconds}"),
            Err(_) => println!("D:ERR"),
        }
    }
}
