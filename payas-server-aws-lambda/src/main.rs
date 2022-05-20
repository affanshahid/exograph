use lambda_http::{Error, Request};

use payas_server_aws_lambda::request_context::LambdaRequestContextProducer;
use payas_server_aws_lambda::resolve;
use payas_server_core::create_operations_executor;
use payas_sql::Database;

use std::sync::Arc;
use std::{env, process::exit};

/// Run the server in production mode with a compiled claypot file
#[tokio::main]
async fn main() -> Result<(), Error> {
    let claypot_file = get_claypot_file_name();

    let database = Database::from_env(None).expect("Failed to access database"); // TODO: error handling here
    let operations_executor =
        Arc::new(create_operations_executor(&claypot_file, database).unwrap());
    let request_context_processor = Arc::new(LambdaRequestContextProducer::new());

    let service = lambda_http::service_fn(|request: Request| async {
        resolve(
            request,
            operations_executor.clone(),
            request_context_processor.clone(),
        )
        .await
    });

    lambda_http::run(service).await?;

    Ok(())
}

fn get_claypot_file_name() -> String {
    let mut args = env::args().skip(1);

    if args.len() > 1 {
        // $ clay-server <model-file-name> extra-arguments...
        println!("Usage: clay-server <claypot-file>");
        exit(1)
    }

    if args.len() == 0 {
        // $ clay-server
        "index.claypot".to_string()
    } else {
        let file_name = args.next().unwrap();

        if file_name.ends_with(".claypot") {
            // $ clay-server concerts.claypot
            file_name
        } else if file_name.ends_with(".clay") {
            // $ clay-server concerts.clay
            format!("{}pot", file_name)
        } else {
            println!("The input file {} doesn't appear to be a claypot. You need build one with the 'clay build <model-file-name>' command.", file_name);
            exit(1);
        }
    }
}