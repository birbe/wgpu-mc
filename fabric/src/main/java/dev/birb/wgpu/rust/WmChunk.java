package dev.birb.wgpu.rust;

import dev.birb.wgpu.palette.RustPalette;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import net.minecraft.client.world.ClientWorld;
import net.minecraft.network.PacketByteBuf;
import net.minecraft.util.collection.IndexedIterable;
import net.minecraft.util.collection.PackedIntegerArray;
import net.minecraft.util.collection.PaletteStorage;
import net.minecraft.util.math.BlockPos;
import net.minecraft.util.math.ChunkPos;
import net.minecraft.util.math.ChunkSectionPos;
import net.minecraft.world.biome.Biome;
import net.minecraft.world.chunk.ChunkNibbleArray;
import net.minecraft.world.chunk.Palette;
import net.minecraft.world.chunk.PalettedContainer;
import net.minecraft.world.chunk.WorldChunk;
import net.minecraft.world.chunk.light.ChunkLightProvider;
import org.lwjgl.PointerBuffer;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;

public class WmChunk {
    private final WorldChunk worldChunk;
    private final int x;
    private final int z;

    public WmChunk(WorldChunk worldChunk) {
        this.x = worldChunk.getPos().x;
        this.z = worldChunk.getPos().z;

        this.worldChunk = worldChunk;
    }

    public void uploadAndBake() throws ClassCastException {
        PointerBuffer paletteIndices = PointerBuffer.allocateDirect(24);
        PointerBuffer storageIndices = PointerBuffer.allocateDirect(24);

        assert this.worldChunk.getSectionArray().length == 24;

        ChunkLightProvider<?, ?> skyLightProvider = worldChunk.getWorld().getLightingProvider().skyLightProvider;
        ChunkLightProvider<?, ?> blockLightProvider = worldChunk.getWorld().getLightingProvider().blockLightProvider;

        ByteBuffer skyBytes = MemoryUtil.memAlloc(2048 * 24);
        ByteBuffer blockBytes = MemoryUtil.memAlloc(2048 * 24);

        ChunkPos pos = this.worldChunk.getPos();

        long grassColorBytes = MemoryUtil.nmemAlignedAlloc(4,16 * 16 * 4);

        for(int x = 0; x < 16; x++) {
            for(int z = 0; z < 16; z++) {
                int color = ((ClientWorld) this.worldChunk.getWorld()).calculateColor(new BlockPos((this.x * 16) + x, 0, (this.z * 16) + z), Biome::getGrassColorAt);
                long offset = ((x * 16) + z) * 4;
                MemoryUtil.memPutInt(grassColorBytes + offset, color);
            }
        }

        for (int i = 0; i < 24; i++) {
            Palette<?> palette;
            PalettedContainer<?> container;
            try {
                palette = this.worldChunk.getSection(i).getBlockStateContainer().data.palette;
                container = this.worldChunk.getSection(i).getBlockStateContainer();
            } catch (ArrayIndexOutOfBoundsException e) {
                continue;
            }

            long sectionPos = ChunkSectionPos.from(pos, i - 4).asLong();

            if(skyLightProvider != null && blockLightProvider != null) {
                ChunkNibbleArray skyNibble = skyLightProvider.lightStorage.uncachedStorage.get(sectionPos);
                ChunkNibbleArray blockNibble = blockLightProvider.lightStorage.uncachedStorage.get(sectionPos);

                if(skyNibble != null) {
                    byte[] sectionSkyLightBytes = skyNibble.asByteArray();
                    skyBytes.put(i * 2048, sectionSkyLightBytes);
                }

                if(blockNibble != null) {
                    byte[] sectionBlockLightBytes = blockNibble.asByteArray();
                    blockBytes.put(i * 2048, sectionBlockLightBytes);
                }
            }

            PaletteStorage paletteStorage = container.data.storage;

            RustPalette rustPalette = new RustPalette(
                    container.idList,
                    WgpuNative.uploadIdList((IndexedIterable<Object>) container.idList)
            );

            ByteBuf buf = Unpooled.buffer(palette.getPacketSize());
            PacketByteBuf packetBuf = new PacketByteBuf(buf);
            palette.writePacket(packetBuf);
            rustPalette.readPacket(packetBuf);

            paletteIndices.put(i, rustPalette.getSlabIndex() + 1);

            if (paletteStorage instanceof PackedIntegerArray array) {
                long index = WgpuNative.createPaletteStorage(
                        paletteStorage.getData(),
                        array.elementsPerLong,
                        paletteStorage.getElementBits(),
                        array.maxValue,
                        array.indexScale,
                        array.indexOffset,
                        array.indexShift,
                        paletteStorage.getSize()
                );

                storageIndices.put(i, index + 1);
            }
        }

        Thread thread = new Thread(() -> {
            WgpuNative.createChunk(this.x, this.z, paletteIndices.address0(), storageIndices.address0(), MemoryUtil.memAddress0(blockBytes), MemoryUtil.memAddress0(skyBytes), grassColorBytes);
            WgpuNative.bakeChunk(this.x, this.z);
        });

        thread.start();

        MemoryUtil.nmemAlignedFree(grassColorBytes);

    }
}
