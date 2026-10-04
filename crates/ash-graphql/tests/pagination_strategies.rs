//! A read's query pages as its action's `pagination` declares, as AshGraphql's list
//! queries: a plain list for a read that doesn't page, a `PageOf<Resource>` for one that
//! pages by offset only, and a `KeysetPageOf<Resource>` for one that pages by keyset,
//! each holding the action's default limit unless asked for another.

use ash_core::{Context, Resource, resource};
use ash_graphql::AshGraphQL;
use ash_memory::Memory;
use async_graphql::Request;
use serde_json::{Value, json};
use uuid::Uuid;

resource! {
    Song {
        table "songs";

        attributes {
            id: Uuid [pk];
            rank: i64;
        }

        actions {
            create create { primary; accept [rank]; }
            read read { primary; }

            read charted {
                pagination offset: true, countable: true, default_limit: 2;
            }

            read latest {
                pagination keyset: true, default_limit: 3, max_page_size: 4;
            }
        }
    }
}

async fn run(ctx: &Context<Memory>, query: &str) -> Value {
    let schema = AshGraphQL::from_resources(&[&Song::DEF]).finish::<Memory>().unwrap();
    let response = schema.execute(Request::new(query).data(ctx.clone())).await;
    assert!(response.errors.is_empty(), "{query}: {:?}", response.errors);
    response.data.into_json().unwrap()
}

fn ranks(songs: &Value) -> Vec<i64> {
    songs.as_array().unwrap().iter().map(|song| song["rank"].as_i64().unwrap()).collect()
}

#[tokio::test]
async fn reads_page_as_their_actions_declare() {
    let ctx = Context::new(Memory::new());
    for rank in 1..=5 {
        Song::create(&ctx).rank(rank).await.unwrap();
    }

    // Unpaged: every record, as a list.
    let all = run(&ctx, "{ listSongs(sort: [{ field: RANK }]) { rank } }").await;
    assert_eq!(ranks(&all["listSongs"]), [1, 2, 3, 4, 5]);

    // By offset: the default limit, and where the page is among the pages.
    let first = run(
        &ctx,
        "{ chartedSongs(sort: [{ field: RANK }]) { results { rank } count limit hasNextPage hasPreviousPage pageNumber lastPage } }",
    )
    .await;
    let page = &first["chartedSongs"];
    assert_eq!(ranks(&page["results"]), [1, 2]);
    assert_eq!(
        (&page["count"], &page["limit"], &page["hasNextPage"], &page["hasPreviousPage"], &page["pageNumber"], &page["lastPage"]),
        (&json!(5), &json!(2), &json!(true), &json!(false), &json!(1), &json!(3))
    );
    let last = run(
        &ctx,
        "{ chartedSongs(sort: [{ field: RANK }], limit: 2, offset: 4) { results { rank } hasNextPage hasPreviousPage pageNumber } }",
    )
    .await;
    let page = &last["chartedSongs"];
    assert_eq!(ranks(&page["results"]), [5]);
    assert_eq!((&page["hasNextPage"], &page["hasPreviousPage"]), (&json!(false), &json!(true)));
    // Uncounted, it's the only page AshGraphql knows of.
    assert_eq!(page["pageNumber"], json!(1));

    // By keyset: the default limit, and never more than the largest page.
    let keyset = run(&ctx, "{ latestSongs(sort: [{ field: RANK }]) { results { rank } endKeyset } }").await;
    assert_eq!(ranks(&keyset["latestSongs"]["results"]), [1, 2, 3]);
    let capped = run(&ctx, "{ latestSongs(sort: [{ field: RANK }], first: 10) { results { rank } } }").await;
    assert_eq!(ranks(&capped["latestSongs"]["results"]), [1, 2, 3, 4]);
}

#[tokio::test]
async fn page_types_follow_the_reads() {
    let schema = AshGraphQL::from_resources(&[&Song::DEF]).finish::<Memory>().unwrap();
    let sdl = schema.sdl();
    assert!(sdl.contains("listSongs(sort: [SongSortInput], filter: SongFilterInput): [Song!]!"), "{sdl}");
    assert!(sdl.contains("limit: Int = 2, offset: Int): PageOfSong"), "{sdl}");
    assert!(sdl.contains("first: Int, before: String, after: String, last: Int): KeysetPageOfSong"), "{sdl}");
    // A page counts where some read of the resource counts.
    let keyset_page = sdl.split("type KeysetPageOfSong {").nth(1).and_then(|rest| rest.split('}').next()).unwrap();
    assert!(keyset_page.contains("count: Int"), "{keyset_page}");
}
