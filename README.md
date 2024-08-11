# lis
Life is short, but data should live forever

## Building
```bash
cargo build
cp ./target/debug/lis ./lis
```

## Usage
**Obs:** you can use `cargo run -- --root ...` instead of `lis` if you want to build and run every time (.e.g. when developing or testing changes).

Put a file in the node at `/path/to/node/directory`
```bash
lis --root /path/to/node/directory put ./my_file.txt
```

List files in the node at `/path/to/node/directory`
```bash
lis --root /path/to/node/directory ls
```

Get contents of `README.md` file in the node at `/path/to/node/directory`
```bash
lis --root /path/to/node/directory put README.md
lis --root /path/to/node/directory get README.md
```




