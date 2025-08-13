# lis
> Life is short, but data should live forever

A distributed filesystem implemented using FUSE and Rust.

## Building
First make sure you have `fuse3` dev installed:
```bash
sudo apt-get install fuse3 libfuse3-dev
```
Then build Lis:
```bash
cargo build
cp ./target/debug/lis ./lis
```

## Usage
**Obs:** you can use `cargo run -- /path/to/root ...` instead of `lis` if you want to build and run every time (.e.g. when developing or testing changes).

Put a file in the node at `/path/to/node/directory`
```bash
lis /path/to/root put ./my_file.txt
```

List files in the node at `/path/to/node/directory`
```bash
lis /path/to/root list
```

Mount FUSE filesystem (readonly)
```bash
# will hang, leave it running
lis /path/to/root mount /path/to/mountpoint

# in another terminal
ls /path/to/mountpoint
cat /path/to/mountpoint/a-file.txt
```

Get contents of `README.md` file in the node at `/path/to/node/directory`
```bash
lis /path/to/root put README.md
lis /path/to/root get README.md
```




