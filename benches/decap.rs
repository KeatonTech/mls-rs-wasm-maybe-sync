use std::collections::HashMap;

use aws_mls::bench_utils::create_empty_tree::{load_test_cases, TestCase};

use criterion::{
    criterion_group, criterion_main, measurement::WallTime, BenchmarkGroup, BenchmarkId, Criterion,
};

use aws_mls::cipher_suite::CipherSuite;

use aws_mls::extension::ExtensionList;

use aws_mls::tree_kem::Capabilities;

use aws_mls::tree_kem::kem::TreeKem;
use aws_mls::tree_kem::node::LeafIndex;
use aws_mls::tree_kem::update_path::ValidatedUpdatePath;

fn decap_setup(c: &mut Criterion) {
    let mut decap_group = c.benchmark_group("decap");

    let cipher_suite = CipherSuite::Curve25519Aes128;

    println!("Benchmarking decap for: {cipher_suite:?}");

    let trees = load_test_cases();

    // Run Decap Benchmark
    bench_decap(&mut decap_group, cipher_suite, trees, &[], None, None);

    decap_group.finish();
}

fn bench_decap(
    bench_group: &mut BenchmarkGroup<WallTime>,
    cipher_suite: CipherSuite,
    map: HashMap<usize, TestCase>,
    added_leaves: &[LeafIndex],
    capabilities: Option<Capabilities>,
    extensions: Option<ExtensionList>,
) {
    for (key, mut value) in map {
        // Perform the encap function
        let encap_gen = TreeKem::new(&mut value.encap_tree, &mut value.encap_private_key)
            .encap(
                b"test_group",
                &mut value.group_context,
                &[],
                &value.encap_signer,
                capabilities.clone(),
                extensions.clone(),
            )
            .unwrap();

        // Apply the update path to the rest of the leaf nodes using the decap function
        let validated_update_path = ValidatedUpdatePath {
            leaf_node: encap_gen.update_path.leaf_node,
            nodes: encap_gen.update_path.nodes,
        };

        // Create one receiver tree so decap is run once
        let mut receiver_tree = value.test_tree.clone();
        let mut private_keys = value.private_keys;

        bench_group.bench_with_input(
            BenchmarkId::new(format!("{cipher_suite:?}"), key),
            &key,
            |b, _| {
                b.iter(|| {
                    TreeKem::new(&mut receiver_tree, &mut private_keys[0])
                        .decap(
                            LeafIndex::new(0),
                            &validated_update_path,
                            added_leaves,
                            &mut value.group_context,
                        )
                        .unwrap();
                })
            },
        );
    }
}

criterion_group!(benches, decap_setup);
criterion_main!(benches);
