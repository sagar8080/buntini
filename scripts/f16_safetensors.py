"""Converts a float32 safetensors file to float16, using only the standard library.

Buntini bundles its memory-search model at half precision: the same results at half
the size. Usage: python3 -I scripts/f16_safetensors.py SOURCE DESTINATION
"""
import json
import struct
import sys
from array import array

CHUNK = 1 << 20  # floats per conversion step, to bound memory use


def convert(source: str, destination: str) -> None:
    with open(source, "rb") as reader:
        (length,) = struct.unpack("<Q", reader.read(8))
        header = json.loads(reader.read(length))
        body = reader.read()
    metadata = header.pop("__metadata__", None)
    out_header, chunks, offset = {}, [], 0
    for name, info in sorted(header.items(), key=lambda item: item[1]["data_offsets"][0]):
        start, end = info["data_offsets"]
        raw = body[start:end]
        if info["dtype"] == "F32":
            floats = array("f")
            floats.frombytes(raw)
            if sys.byteorder != "little":
                floats.byteswap()
            parts = [struct.pack(f"<{len(floats[i:i + CHUNK])}e", *floats[i:i + CHUNK]) for i in range(0, len(floats), CHUNK)]
            raw, dtype = b"".join(parts), "F16"
        else:
            dtype = info["dtype"]
        out_header[name] = {"dtype": dtype, "shape": info["shape"], "data_offsets": [offset, offset + len(raw)]}
        chunks.append(raw)
        offset += len(raw)
    if metadata is not None:
        out_header["__metadata__"] = metadata
    encoded = json.dumps(out_header, separators=(",", ":"), sort_keys=True).encode()
    encoded += b" " * (-len(encoded) % 8)
    with open(destination, "wb") as writer:
        writer.write(struct.pack("<Q", len(encoded)))
        writer.write(encoded)
        for raw in chunks:
            writer.write(raw)


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("Usage: f16_safetensors.py SOURCE DESTINATION")
    convert(sys.argv[1], sys.argv[2])
