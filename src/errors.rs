error_chain! {
    types {
        Error, ErrorKind, ResultExt, Result;
    }

    errors {
        Connection(msg: String) {
            description("Connection error")
            display("Connection error: {}", msg)
        }

        RpcError(code: i64, error: serde_json::Value, method: String) {
            description("RPC error")
            display("{} RPC error {}: {}", method, code, error)
        }

        Interrupt(sig: i32) {
            description("Interruption by external signal")
            display("Iterrupted by signal {}", sig)
        }

        TooPopular {
            description("Too many history entries")
            display("Too many history entries")
        }
    }
}


