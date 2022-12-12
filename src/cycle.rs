use petgraph::Direction::{Incoming, Outgoing};
use std::collections::HashMap;
use std::io::Write;
use std::sync::atomic::{AtomicU32, Ordering};

use crate::block::Block;
use crate::graph::{MappedCondensedGraph, MappedGraph};

static COUNTER: AtomicU32 = AtomicU32::new(0);

pub fn condensate_graph(
    mut original_graph: MappedGraph,
    entry_node_latency_map: &mut HashMap<u64, u32>,
    blocks: &HashMap<u64, Block>,
) -> MappedCondensedGraph {
    let mut condensed_graph = original_graph.condense_cycles();

    COUNTER.fetch_add(1, Ordering::Relaxed);

    for condensed_node in condensed_graph.get_condensed_nodes() {
        // create new graph with the blocks of the condensed node, acyclic
        let mut cycle_graph = MappedGraph::new();

        // add edges to the cycle_graph
        for block in condensed_node.iter() {
            for target in block.get_targets() {
                if !condensed_node
                    .iter()
                    .filter(|node| node.leader == target)
                    .collect::<Vec<_>>()
                    .is_empty()
                {
                    let target_block = blocks.get(&target).unwrap();
                    cycle_graph.add_edge(
                        block.clone(),
                        target_block.clone(),
                        target_block.get_latency() as f32,
                    );
                }
            }
        }

        //finding the exit node of the cycle to remove the outer node and the associated edge --> prepare the graph for the longest path computation
        let outer_blocks =
            condensed_graph.neighbors_directed(&condensed_node, Outgoing)[0].to_owned(); // we assume that the outer node of a cycle is only one

        let mut outer_block = outer_blocks[0].clone(); // we assume that the outer node of a cycle is only one

        //handle case where outer block has more than one block --> it is a condensed node
        for block in outer_blocks {
            if !cycle_graph
                .get_nodes()
                .iter()
                .filter(|node| node.get_targets().contains(&block.leader))
                .collect::<Vec<_>>()
                .is_empty()
            {
                outer_block = block;
            }
        }

        let exit_cycle_blocks = &original_graph.neighbors_directed(&outer_block, Incoming); //we should check if the outer node is condensed
        let mut exit_block = &exit_cycle_blocks[0]; // to initialize the variable

        for exit_cycle_block in exit_cycle_blocks {
            if !cycle_graph
                .get_nodes()
                .iter()
                .filter(|node| node.leader == exit_cycle_block.leader)
                .collect::<Vec<_>>()
                .is_empty()
            {
                exit_block = exit_cycle_block;
               // cycle_graph.remove_edge(&exit_block, &outer_block);
               // cycle_graph.remove_node(&outer_block);
            }
        }

        println!("exit_block: {:x}", exit_block.leader);

        //let cycle_entry_block = condensed_node[0].clone(); // entry node is always the first block //FALSE

        //finding the entry node of the cycle
        let outer_blocks =
            condensed_graph.neighbors_directed(&condensed_node, Incoming)[0].to_owned(); // it is not important which block we take, we just need one

        let mut outer_block = outer_blocks[0].clone(); // we assume that the outer node of a cycle is only one

        //handling case where outer block has more than one block --> it is a condensed node
        for block in outer_blocks {
            for condensed_block in &condensed_node {
                if block.get_targets().contains(&condensed_block.leader) {
                    outer_block = block.clone();
                }
            }
        }

        let cycle_entry_blocks = &original_graph.neighbors_directed(&outer_block, Outgoing);
        let mut entry_block = &cycle_entry_blocks[0]; // to initialize the variable

        for cycle_entry_block in cycle_entry_blocks {
            for (source, target, _) in cycle_graph.edges_directed(&cycle_entry_block, Incoming) {
                cycle_graph.remove_edge(&source, &target);
                entry_block = cycle_entry_block;
            }
        }

        //print cycle_entry_block
        println!("cycle_entry_block: {:x}", entry_block.leader);

        let digraph = cycle_graph.to_dot_graph();
        let mut dot_file = std::fs::File::create(format!(
            "graph_cycle_{}.dot",
            COUNTER.load(Ordering::Relaxed)
        ))
        .expect("Unable to create file");
        dot_file
            .write_all(digraph.as_bytes())
            .expect("Unable to write dot file");

        let entry_node_latency = entry_block.get_latency();

        match cycle_graph.reconstruct_longest_path(
            entry_block,
            entry_block,
            exit_block,
            entry_node_latency as f32,
        ) {
            Ok(cycle_node_latency) => {
                let node_incoming_edges = condensed_graph.edges_directed(&condensed_node, Incoming);

                if node_incoming_edges.is_empty() {
                    // if the node has no incoming edges, it is an entry node
                    entry_node_latency_map
                        .insert(condensed_node[0].leader, cycle_node_latency as u32);
                // we chose [0] as reference for the condensed node for simplicity
                } else {
                    for (source, target, _) in node_incoming_edges {
                        condensed_graph.update_edge(&source, &target, cycle_node_latency as f32);
                    }
                    entry_node_latency_map
                        .insert(condensed_node[0].leader, condensed_node[0].get_latency());
                }
            }
            Err(_) => {
                let mut condensed_cycle_graph =
                    condensate_graph(cycle_graph.clone(), entry_node_latency_map, blocks);

                let condensed_cycle_graph_nodes = condensed_cycle_graph.get_nodes();
                let entry_nodes = condensed_cycle_graph_nodes
                    .iter()
                    .filter(|node| {
                        condensed_cycle_graph
                            .edges_directed(node, Incoming)
                            .is_empty()
                    })
                    .collect::<Vec<_>>();

                let condensed_cycle_entry_node = entry_nodes[0].clone(); // at this point we are sure that there is only one entry node

                let entry_node_latency =
                    match entry_node_latency_map.get(&condensed_cycle_entry_node[0].leader) {
                        // now we are sure that if the entry node is a condensed one , its latency is already in the map
                        Some(latency) => *latency as u32,
                        None => condensed_cycle_entry_node[0].get_latency(),
                    };

                // get the outer block of the cyclic node (it's always only one because it's the exit condition of the cycle)
                let outer_blocks =
                    condensed_graph.neighbors_directed(&condensed_node, Outgoing)[0].to_owned();

                let mut outer_block = outer_blocks[0].clone(); // to initialize the variable

                //handle case where outer block has more than one block --> it is a condensed node
                for block in outer_blocks {
                    if !condensed_cycle_graph
                        .get_nodes()
                        .iter()
                        .filter(|node| node[0].get_targets().contains(&block.leader)) //[0] beacuse the exit node is surely not a condensed node
                        .collect::<Vec<_>>()
                        .is_empty()
                    {
                        outer_block = block;
                    }
                }
                // get the cycle exit block in the original graph
                let exit_block = &original_graph.neighbors_directed(&outer_block, Incoming);

                //find last block of condensated cycle graph
                let mut last_block = condensed_cycle_entry_node.clone(); // we assume the last block is not condensed
                let mut last_block_found = false;
                while !last_block_found {
                    let neighbors = condensed_cycle_graph.neighbors_directed(&last_block, Outgoing);
                    if neighbors.is_empty() {
                        last_block_found = true;
                    } else {
                        last_block = neighbors[0].clone();
                    }
                }

                let cycle_node_latency = condensed_cycle_graph
                    .reconstruct_longest_path(
                        &condensed_cycle_entry_node,
                        &condensed_cycle_entry_node[0],
                        exit_block,
                        &last_block,
                        entry_node_latency as f32,
                    )
                    .unwrap();

                let node_incoming_edges = condensed_graph.edges_directed(&condensed_node, Incoming);
                if node_incoming_edges.is_empty() {
                    // if the node has no incoming edges, it is an entry node
                    entry_node_latency_map
                        .insert(condensed_node[0].leader, cycle_node_latency as u32);
                // we chose [0] as reference for the condensed node for simplicity
                } else {
                    for (source, target, _) in node_incoming_edges {
                        condensed_graph.update_edge(&source, &target, cycle_node_latency as f32);
                    }
                    entry_node_latency_map
                        .insert(condensed_node[0].leader, condensed_node[0].get_latency());
                }

                let digraph = condensed_cycle_graph.to_dot_graph();
                let mut dot_file = std::fs::File::create(format!(
                    "condensed_cycle_graph_{}.dot",
                    COUNTER.load(Ordering::Relaxed)
                ))
                .expect("Unable to create file");
                dot_file
                    .write_all(digraph.as_bytes())
                    .expect("Unable to write dot file");
            }
        }
    }

    return condensed_graph;
}
