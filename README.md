# lis
Life is short, but data should live forever

## Building
```bash
cargo build
cp ./target/debug/lis ./lis
```

## Usage
**Obs:** you can use `cargo run -- --root ...` instead of `lis` if you want to build and run every time (.e.g. when developing or testing changes).

Add a file to the node at `/path/to/node/directory`
```bash
lis --root /path/to/node/directory add ./my_file.txt
```

List files in the node at `/path/to/node/directory`
```bash
lis --root /path/to/node/directory ls
```




