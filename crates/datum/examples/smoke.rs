use grid_datum::normalize;

fn d(src: &str) -> String {
    normalize(src).unwrap().datum.to_string()
}

fn main() {
    // formatting + comments + local renaming are invisible
    let a = "def f(x):\n    y = x + 1  # bump\n    return y\n";
    let b = "def f(x):\n\n    # bump\n    total    =   x+1\n    return total\n";
    println!("rename/format invariant: {}", d(a) == d(b));
    // interface names are identity
    let c = "def g(x):\n    y = x + 1\n    return y\n";
    println!("fn name matters: {}", d(a) != d(c));
    // globals are not locals
    let g1 = "def f():\n    global count\n    count = 1\n";
    let g2 = "def f():\n    global other\n    other = 1\n";
    println!("global name matters: {}", d(g1) != d(g2));
    // comprehension scope
    let h1 = "def f(xs):\n    return [i*2 for i in xs]\n";
    let h2 = "def f(ys):\n    return [j*2 for j in ys]\n";
    println!("comprehension invariant: {}", d(h1) == d(h2));
    // class attributes are interface
    let k1 = "class C:\n    attr = 1\n";
    let k2 = "class C:\n    other = 1\n";
    println!("class attr matters: {}", d(k1) != d(k2));
    // idempotence of datum over repeated runs
    println!("determinism: {}", d(a) == d(a));
    // comments retained as metadata
    println!("comments: {:?}", normalize(a).unwrap().comments);
    println!("datum: {}", d(a));
}
