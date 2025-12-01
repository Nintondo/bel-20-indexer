## How to

1. Rename `.env.example` to `.env`
2. Replace the values in .env with your own values
3. Make sure you have installed rust and cargo
4. Run the following command to start the server

```bash
cargo r -r
```
RUSTFLAGS="-C target-cpu=native" 

### Steps to index using blk files (faster in 5-20 times)

1. __BLK_DIR__: Set this to the path containing your Dogecoin/Bellscoin blockchain data files (blk*.dat files), typically found at `/home/<user>/.dogecoin/blocks` or `/home/<user>/.bells/blocks`.
2. __INDEX_DIR__: This requires a copy of the LevelDB index from your Dogecoin node.

⚠️ __Before copying the index__:
- Shut down your Dogecoin node completely to prevent data corruption
- Use rsync to create a copy of the index directory

Example of safely copying the index:
```
# First, stop the Dogecoin node
dogecoin-cli stop
# or if using systemd
sudo systemctl stop dogecoind

# Wait a moment to ensure the node is fully stopped
sleep 5

# Copy the index to your project directory
rsync -av --delete /home/<user>/.dogecoin/blocks/index/ /path/to/your/project/index/

# After copying is complete, you can restart your Dogecoin node if needed
# dogecoin-cli -daemon
# or
# sudo systemctl start dogecoind
```

## API Documentation

### Overview
This API provides endpoints to interact with addresses, tokens, and events. Below are the available routes, their parameters, and descriptions.

### Routes

#### GET /address/:address
 - __Description__: Retrieves token balances and transfers for a specific address.
 - __Parameters__:
   - address (path): The address to retrieve token balances and transfers for.

##### Response example:
```json
[
    {
        "tick": "<tick>",
        "balance": "1000",
        "transferable_balance": "1000",
        "transfers": [
            {
                "outpoint": "<txid:vout>",
                "value": "1000"
            }
        ],
        "transfers_count": 1
    },
    ...
]
```


#### GET /address/:address/history
 - __Description__: Retrieves the history of token actions for a specific address.
 - __Parameters__:
   - __address__ (path): The address to retrieve token history for.
   - __tick__ (query): The token tick to filter by.
   - __offset__ (query, optional): The offset for pagination. (key: `id`)
   - __limit__ (query, optional): The maximum number of records to return.

##### Response example:
```json
[
    {
        "id": 1,
        "tick": "<tick>",
        "height": 100,
        "type": "Send",
        "amt": "1",
        "recipient": "<address>",
        "adddress": "<address>",
        "txid": "<txid>",
        "vout": 0,
        "created": 198773477
    },
    ...
]
```

#### GET /events/:height
 - __Description__: Retrieves the history of token actions for a specific height.
 - __Parameters__:
   - __height__ (path): The block number to retrieve events history for.

##### Response example:
```json
[
    {
        "id": 1,
        "tick": "<tick>",
        "height": 100,
        "type": "Send",
        "amt": "1",
        "recipient": "<address>",
        "adddress": "<address>",
        "txid": "<txid>",
        "vout": 0
    },
    ...
]
```

#### GET /txid/:txid
 - __Description__: Retrieves events by TXID
 - __Parameters__:
   - __txid__ (path): Transaction Hash (ID)

##### Response example:
```json
[
    {
        "id": 1,
        "tick": "<tick>",
        "height": 100,
        "type": "Send",
        "amt": "1",
        "recipient": "<address>",
        "adddress": "<address>",
        "txid": "<txid>",
        "vout": 0
    },
    ...
]
```


#### GET /tokens
 - __Description__: Retrieves metadata for all tokens.

##### Response example:
```json
[
    {
        "genesis": "<inscription_id>",
        "tick": "<tick>",
        "max": "1000000000",
        "lim": "1000",
        "dec": 18,
        "supply": "6000",
        "mint_count": 5,
        "transfer_count": 10,
        "holders": 10
    },
    ...
]
```

#### POST /events
 - __Description__: Subscribes to events related to specific addresses and tokens.
 - Parameters:
   - __addresses__ (body, optional): A set of addresses to subscribe to.
   - __tokens__ (body, optional): A set of tokens to subscribe to.

##### Response examples:


###### New block
```json
{
  "event_type": "new_block",
  "height": 1,
  "proof": "<hash>",
  "blockhash": "<hash>"
}
```

###### Reorg
```json
{
  "event_type": "reorg",
  "blocks_count": 5,
  "new_height": 67890
}
```
###### Deploy
```json
{
  "id": 1,
  "type": "Deploy",
  "max": "1000000",
  "lim": "500000",
  "dec": 18,
  "address": "<address>",
  "txid": "<txid>",
  "vout": 0
}
```

###### Mint
```json
{
  "id": 1,
  "type": "Mint",
  "amt": "1000.0",
  "address": "<address>",
  "txid": "<txid>",
  "vout": 0
}
```

###### DeployTransfer
```json
{
  "id": 1,
  "type": "DeployTransfer",
  "amt": "500.0",
  "address": "<address>",
  "txid": "<txid>",
  "vout": 0
}
```

###### Send
```json
{
  "id": 1,
  "type": "Send",
  "amt": "250.0",
  "recipient": "<address>",
  "address": "<address>",
  "txid": "<txid>",
  "vout": 0
}
```

###### Receive
```json
{
  "id": 1,
  "type": "Receive",
  "amt": "250.0",
  "sender": "<address>",
  "address": "<address>",
  "txid": "<txid>",
  "vout": 0
}
```

###### SendReceive
```json
{
  "id": 1,
  "type": "SendReceive",
  "amt": "500.0",
  "address": "<address>",
  "txid": "<txid>",
  "vout": 0
}
```


#### GET /status
 - __Description__: Retrieves current status of the server

##### Response example:
```json
{
    "height": 0,
    "proof": "<hash>",
    "blockhash": "<hash>"
}
```

#### GET /proof-of-history
 - __Description__:
 - Parameters:
   - __offset__ (query, optional): The offset for pagination. (key: `height`)
   - __limit__ (query, optional): The maximum number of records to return. (up to 100)

##### Response example:
```json
[
    {
        "height": 0,
        "hash": "<hash>"
    },
    ...
]
```
