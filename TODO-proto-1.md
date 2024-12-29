- add File type (filename, hash on files doc, size, content (bytes))
- integrate get file
- integrate remove file
- implement add dir
- join network through node x


architecture
    - only one document (root document)
        filename: blob hash
    - updating a file means readding it as a new blob and resetting the root document to point to it


- fuse
- waku for message passing
- architecture: https://excalidraw.com/#json=Z5wK1wkZrwUyeAbpT_oRE,OYn8RCSqbvIsuLdZFqXFhw
