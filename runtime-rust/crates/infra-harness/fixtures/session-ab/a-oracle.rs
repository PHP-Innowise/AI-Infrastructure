mod candidate { include!("/input/source.rs"); }
fn main() {
    for values in [vec![], vec![1], vec![1,1,1], vec![1,2,1], vec![-1,-1,0,0,0,2], vec![i32::MIN,i32::MIN,i32::MAX]] {
        println!("C:{:?}", candidate::compress_runs(&values));
    }
    for (runs, limit) in [(vec![],0), (vec![(3,2),(-1,1)],3), (vec![(3,2)],1), (vec![(0,0)],1), (vec![(1,usize::MAX),(2,1)],usize::MAX)] {
        match candidate::expand_runs(&runs, limit) {
            Ok(values) => println!("E:{values:?}"),
            Err(_) => println!("E:ERR"),
        }
    }
    let original = vec![2,2,3,3,2];
    println!("R:{}", candidate::expand_runs(&candidate::compress_runs(&original), original.len()).ok() == Some(original));
}
