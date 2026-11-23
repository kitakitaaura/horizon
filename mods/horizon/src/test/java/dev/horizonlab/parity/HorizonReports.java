package dev.horizonlab.parity;

import java.nio.file.Path;
import java.util.concurrent.CompletableFuture;
import net.minecraft.SharedConstants;
import net.minecraft.core.HolderLookup;
import net.minecraft.data.DataGenerator;
import net.minecraft.data.info.BiomeParametersDumpReport;
import net.minecraft.data.info.CommandsReport;
import net.minecraft.data.info.RegistryComponentsReport;
import net.minecraft.data.info.RegistryDumpReport;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.server.jsonrpc.dataprovider.JsonRpcApiSchema;
import net.minecraft.util.Util;

/**
 * Vanilla's data generator reports ({@code net.minecraft.data.Main --reports}), limited to those Horizon
 * reads: biome parameters, item default components, the command tree and the registry dump, plus the
 * JSON-RPC schema {@code tools/test_management_protocol_parity.py} compares against.
 * Each is the provider {@code Main} itself would run, wired the way {@code Main} wires it, so the files
 * are the same; the reports Horizon has no use for (the block list, packets, the data pack structure)
 * are skipped. The block list alone is most of the generator's time on a slow JVM.
 *
 * <p>Usage: {@code HorizonReports <output>}, the folder {@code Main --output} would get. Run in
 * {@code PrepareWorker}, which has bootstrapped the game already.
 */
public final class HorizonReports {
    private HorizonReports() {}

    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            throw new IllegalArgumentException("usage: HorizonReports <output>");
        }
        SharedConstants.tryDetectVersion();
        // Loading DataGenerator bootstraps the game's registries, as it does for Main.
        DataGenerator generator =
                new DataGenerator.Cached(Path.of(args[0]), SharedConstants.getCurrentVersion(), true);
        CompletableFuture<HolderLookup.Provider> registries =
                CompletableFuture.supplyAsync(VanillaRegistries::createWorldLookup, Util.backgroundExecutor());
        DataGenerator.PackGenerator pack = generator.getVanillaPack(true);
        pack.addProvider(output -> new BiomeParametersDumpReport(output, registries));
        pack.addProvider(output -> new RegistryComponentsReport(output, registries));
        pack.addProvider(output -> new CommandsReport(output, registries));
        pack.addProvider(RegistryDumpReport::new);
        pack.addProvider(JsonRpcApiSchema::new);
        // No Util.shutdownExecutors(), unlike Main: in PrepareWorker the exporters after this one
        // still use vanilla's executors, and the worker exits the JVM when its tasks are done.
        generator.run();
    }
}
