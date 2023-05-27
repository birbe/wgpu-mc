package dev.birb.wgpu;

import com.google.gson.Gson;
import dev.birb.wgpu.palette.RustPalette;
import dev.birb.wgpu.render.electrum.ElectrumRenderer;
import dev.birb.wgpu.rust.WgpuNative;
import net.fabricmc.api.ClientModInitializer;
import net.fabricmc.api.ModInitializer;
import net.fabricmc.fabric.api.client.event.lifecycle.v1.ClientTickEvents;
import net.fabricmc.fabric.api.client.keybinding.v1.KeyBindingHelper;
import net.fabricmc.fabric.api.renderer.v1.Renderer;
import net.fabricmc.fabric.api.renderer.v1.RendererAccess;
import net.fabricmc.fabric.impl.client.indigo.Indigo;
import net.fabricmc.fabric.impl.client.indigo.IndigoMixinConfigPlugin;
import net.fabricmc.fabric.impl.renderer.RendererAccessImpl;
import net.minecraft.block.BlockState;
import net.minecraft.client.option.KeyBinding;
import net.minecraft.client.render.entity.model.EntityModels;
import net.minecraft.client.util.InputUtil;
import net.minecraft.util.collection.PackedIntegerArray;
import net.minecraft.world.chunk.Chunk;

import net.minecraft.world.chunk.PalettedContainer;
import org.lwjgl.glfw.GLFW;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

public class WgpuMcMod implements ClientModInitializer {

	public static Logger LOGGER = LoggerFactory.getLogger("electrum");

	public static boolean ENTITIES_UPLOADED = false;
	public static boolean MAY_INJECT_PART_IDS = false;

	public static long TIME_SPENT_ENTITIES = 0;
	public static long ENTRIES = 0;

	public static ElectrumRenderer ELECTRUM;

	@Override
	public void onInitializeClient() {
		LOGGER.info("Registering FRAPI renderer");

		ElectrumRenderer electrumRenderer = new ElectrumRenderer();
		RendererAccess.INSTANCE.registerRenderer(electrumRenderer);
	}
}
