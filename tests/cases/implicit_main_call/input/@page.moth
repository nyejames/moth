-- Test calling imported initialization entrypoints explicitly.
import @app_config {init_config, get_database_url}

io.line("Before config initialization")
init_config()
io.line("After config initialization")

database_url = get_database_url()
io.line([: Database URL: [database_url]])
