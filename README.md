Python vs Bun vs Go vs Rust Performance Comparison

# 1. stub
cd stub && go run .

# 2. migrate
DATABASE_URL=
./migrations/migrate.sh

# 3a. Go server
cd servers/go
go mod tidy
set -x STUB_URL http://localhost:9090/v1/chat/completions go run .
# with DB:  go run . -db

# 3b. Bun server
cd servers/bun
bun install
set -x STUB_URL http://localhost:9090/v1/chat/completions 
bun main.ts
# with DB:  DATABASE_URL=... bun main.ts --db

# 4. k6
k6 run k6/baseline.js