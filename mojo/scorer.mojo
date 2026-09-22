from std.python import Python

# JSON plumbing uses CPython; the bounded float64 numerical kernel is Mojo.
# Text and identifiers are excluded by the Rust adapter's numerical protocol.
def main() raises:
    var sys = Python.import_module("sys")
    var json = Python.import_module("json")
    var builtins = Python.import_module("builtins")
    while True:
        var line = sys.stdin.readline(2097152)
        if Int(py=builtins.len(line)) == 0:
            break
        var request = json.loads(line)
        var weights = request["weights"]
        var rows = request["features"]
        var count = Int(py=builtins.len(rows))
        if count < 1 or count > 4096 or Int(py=builtins.len(weights)) != 17:
            raise Error("invalid_shape")
        var w = List[Float64]()
        for j in range(17):
            w.append(Float64(py=weights[j]))
        var scores = builtins.list()
        for i in range(count):
            if Int(py=builtins.len(rows[i])) != 17:
                raise Error("invalid_shape")
            var total: Float64 = 0.0
            for j in range(17):
                total += Float64(py=rows[i][j]) * w[j]
            scores.append(total)
        _ = sys.stdout.write(json.dumps(scores, allow_nan=False) + "\n")
        _ = sys.stdout.flush()
