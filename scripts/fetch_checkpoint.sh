#!/bin/bash
curl -H "Content-Type: application/json" -d '{
    "jsonrpc":"2.0",
    "id":1,
    "method":"ob_fetchCheckpoint",
    "params":["0x8dbc80daefae33d5b9b7d4721ecad1101fc29c3d5a655faa774ea73dd1575749"]
}' http://localhost:9944
