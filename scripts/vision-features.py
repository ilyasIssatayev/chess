"""Expose a pinned ONNX classifier's penultimate features without changing weights.

Only protobuf framing is needed: ModelProto.graph is field 7 and
GraphProto.output is field 12. Source hashes pin the exact graph/tensor name.
This keeps the ordinary installer dependency-free (no training framework).
"""


def varint(value):
    result = bytearray()
    while value > 127:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


def field(number, body):
    return varint((number << 3) | 2) + varint(len(body)) + body


def read_varint(body, offset):
    value = shift = 0
    while offset < len(body) and shift < 70:
        byte = body[offset]
        offset += 1
        value |= (byte & 127) << shift
        if byte < 128:
            return value, offset
        shift += 7
    raise ValueError('Invalid protobuf varint')


def expose_features(body, tensor_name, dimension):
    name = tensor_name.encode()
    # ValueInfoProto -> TypeProto -> Tensor (FLOAT, [batch, dimension]).
    shape = field(1, field(2, b'batch')) + field(1, b'\x08' + varint(dimension))
    tensor_type = b'\x08\x01' + field(2, shape)
    output = field(1, name) + field(2, field(1, tensor_type))
    result = bytearray()
    offset = graphs = 0
    while offset < len(body):
        start = offset
        tag, offset = read_varint(body, offset)
        wire = tag & 7
        if wire == 2:
            size, offset = read_varint(body, offset)
            end = offset + size
        elif wire == 0:
            _, end = read_varint(body, offset)
        elif wire in (1, 5):
            end = offset + (8 if wire == 1 else 4)
        else:
            raise ValueError('Unsupported protobuf wire type')
        if end > len(body):
            raise ValueError('Truncated ONNX model')
        if tag == (7 << 3 | 2):
            graph = body[offset:end]
            if field(2, name) not in graph:
                raise ValueError('Pinned feature tensor not found in the model')
            result.extend(field(7, graph + field(12, output)))
            graphs += 1
        else:
            result.extend(body[start:end])
        offset = end
    if graphs != 1:
        raise ValueError('Expected exactly one ONNX graph')
    return bytes(result)
