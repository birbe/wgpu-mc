package dev.birb.wgpu.mixin.render;

import com.mojang.blaze3d.systems.RenderSystem;
import dev.birb.wgpu.entity.DummyVertexConsumer;
import dev.birb.wgpu.rust.WgpuNative;
import it.unimi.dsi.fastutil.objects.ObjectArrayList;
import net.minecraft.client.MinecraftClient;
import net.minecraft.client.network.ClientPlayerEntity;
import net.minecraft.client.render.*;
import net.minecraft.client.render.BackgroundRenderer.FogType;
import net.minecraft.client.render.chunk.ChunkBuilder;
import net.minecraft.client.render.entity.EntityRenderDispatcher;
import net.minecraft.client.util.math.MatrixStack;
import net.minecraft.client.world.ClientWorld;
import net.minecraft.entity.Entity;
import net.minecraft.entity.LivingEntity;
import net.minecraft.resource.ResourceManager;
import net.minecraft.util.math.MathHelper;
import net.minecraft.util.math.Vec3d;
import net.minecraft.world.tick.TickManager;
import org.jetbrains.annotations.Nullable;
import org.joml.Matrix4f;
import org.joml.Vector3d;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.nio.ByteBuffer;
import java.util.List;
import java.util.Objects;

@Mixin(WorldRenderer.class)
public abstract class WorldRendererMixin {

    @Shadow
    public abstract void updateChunks(Camera camera);

    @Shadow @Final private MinecraftClient client;

    @Shadow protected abstract void setupTerrain(Camera camera, Frustum frustum, boolean hasForcedFrustum, boolean spectator);

    @Shadow private Frustum frustum;

    @Shadow private @Nullable Frustum capturedFrustum;

    @Shadow private @Nullable BuiltChunkStorage chunks;


    @Shadow private boolean shouldCaptureFrustum;

    @Shadow @Final private Vector3d capturedFrustumPosition;

    @Shadow private @Nullable ClientWorld world;

    @Shadow @Final private EntityRenderDispatcher entityRenderDispatcher;


    @Shadow @Final ObjectArrayList<ChunkBuilder.BuiltChunk> field_45616 = new ObjectArrayList(10000);

    @Shadow protected abstract void renderEntity(Entity entity, double cameraX, double cameraY, double cameraZ, float tickDelta, MatrixStack matrices, VertexConsumerProvider vertexConsumers);

    @Shadow private int ticks;

    @Shadow protected abstract void captureFrustum(Matrix4f positionMatrix, Matrix4f projectionMatrix, double x, double y, double z, Frustum frustum);

    @Shadow protected abstract BufferBuilder.BuiltBuffer renderStars(BufferBuilder buffer);
    /**
     * @author wgpu-mc
     * @reason replaced with wgpu equivalent
     */
    @Overwrite
    private void renderLightSky() {

    }

    /**
     * @author wgpu-mc
     * @reason replaced with wgpu equivalent
     */
    @Overwrite
    private void renderDarkSky() {

    }

    /**
     * @author wgpu-mc
     * @reason replaced with wgpu equivalent
     */
    @Overwrite
    private void renderStars() {
        BufferBuilder.BuiltBuffer stars = renderStars(Tessellator.getInstance().getBuffer());
        ByteBuffer vertexBuffer = stars.getVertexBuffer();
        int count = stars.getParameters().vertexCount();

        byte[] bytes = new byte[count * VertexFormats.POSITION.getVertexSizeByte()];
        vertexBuffer.get(bytes);

        int[] quadIndices = new int[count * 6];

        for (int i = 0; i < count; i++) {
            quadIndices[(i * 6)] = i * 4;
            quadIndices[(i * 6) + 1] = (i * 4) + 1;
            quadIndices[(i * 6) + 2] = (i * 4) + 3;
            quadIndices[(i * 6) + 3] = (i * 4) + 1;
            quadIndices[(i * 6) + 4] = (i * 4) + 2;
            quadIndices[(i * 6) + 5] = (i * 4) + 3;
        }

        WgpuNative.bindStarData(count + (count / 2), quadIndices, bytes);
    }

    /**
     * @author wgpu-mc
     * @reason do no such thing
     */
    @Overwrite
    public void reload(ResourceManager manager) {
    }

    @Inject(method = "setupTerrain", cancellable = true, at = @At(value = "INVOKE", shift = At.Shift.AFTER,
    target = "Lnet/minecraft/client/render/BuiltChunkStorage;updateCameraPosition(DD)V"))
    public void setAllVisible(CallbackInfo ci){
        this.field_45616.clear();
        this.field_45616.addAll(new ObjectArrayList(this.chunks.chunks));
    }

    @Redirect(method = "setupTerrain",
    at = @At(value = "INVOKE", target = "Lnet/minecraft/client/render/ChunkRenderingDataPreparer;method_52834(ZLnet/minecraft/client/render/Camera;Lnet/minecraft/client/render/Frustum;Ljava/util/List;)V"))
    public void disableMcCulling(ChunkRenderingDataPreparer c,boolean bl, Camera camera, Frustum frustum, List<ChunkBuilder.BuiltChunk> list){
        
    }

    @Inject(method = "render", cancellable = true, at = @At("HEAD"))
    public void render(MatrixStack matrices, float tickDelta, long limitTime, boolean renderBlockOutline, Camera camera, GameRenderer gameRenderer, LightmapTextureManager lightmapTextureManager, Matrix4f projectionMatrix, CallbackInfo ci) {
        Vec3d translate = camera.getPos();

        TickManager manager = this.client.world.getTickManager();
        float tickChange = manager.shouldTick() ? tickDelta : 1.0F;
        
        BackgroundRenderer.render(camera, tickDelta, this.world, this.client.options.getClampedViewDistance(), gameRenderer.getSkyDarkness(tickDelta));
        BackgroundRenderer.applyFogColor();
        
        Frustum currentFrustum;
        if (this.capturedFrustum != null) {
            currentFrustum = this.capturedFrustum;
            currentFrustum.setPosition(this.capturedFrustumPosition.x, this.capturedFrustumPosition.y, this.capturedFrustumPosition.z);
        } else {
            currentFrustum = this.frustum;
        }

        if (this.shouldCaptureFrustum) {
            captureFrustum(matrices.peek().getPositionMatrix(), projectionMatrix, translate.x, translate.y, translate.z, this.capturedFrustum != null ? new Frustum(matrices.peek().getPositionMatrix(), projectionMatrix) : currentFrustum);
            this.shouldCaptureFrustum = false;
        } 
        
        Objects.requireNonNull(this.world).runQueuedChunkUpdates();
        this.world.getChunkManager().getLightingProvider().doLightUpdates();

        this.setupTerrain(camera, currentFrustum, this.capturedFrustum != null, this.client.player != null && this.client.player.isSpectator());
        this.updateChunks(camera);

        //Todo: Capture terrain fog data as well
        boolean thickFog = (this.client.world.getDimensionEffects().useThickFog(MathHelper.floor(camera.getPos().getX()), MathHelper.floor(camera.getPos().getY())) || this.client.inGameHud.getBossBarHud().shouldThickenFog());
        BackgroundRenderer.applyFog(camera, FogType.FOG_SKY, gameRenderer.getViewDistance(), thickFog, tickChange);
        //BackgroundRenderer.applyFogColor();

        bindSkyData(matrices, projectionMatrix, tickDelta, camera);
        float[] fogColorOverride = this.world.getDimensionEffects().getFogColorOverride(this.world.getSkyAngle(tickDelta), tickDelta);
        bindRenderEffectsData(fogColorOverride);
        // -- Camera --

        MatrixStack cameraStack = new MatrixStack();
        cameraStack.loadIdentity();

        ClientPlayerEntity player = MinecraftClient.getInstance().player;

        // -- Entities --

//        this.blockEntityRenderDispatcher.configure(this.world, camera, this.client.crosshairTarget);
        this.entityRenderDispatcher.configure(this.world, camera, this.client.targetedEntity);

        if(this.world != null) {
            MatrixStack entityStack = new MatrixStack();
            entityStack.loadIdentity();
            VertexConsumerProvider dummyProvider = layer -> new DummyVertexConsumer();

            for(Entity entity : this.world.getEntities()) {
                if((entity != camera.getFocusedEntity() || camera.isThirdPerson() || camera.getFocusedEntity() instanceof LivingEntity && ((LivingEntity)camera.getFocusedEntity()).isSleeping()) && (!(entity instanceof ClientPlayerEntity) || camera.getFocusedEntity() == entity)) {
//                    this.renderEntity(entity, translate.getX(), translate.getY(), translate.getZ(), tickDelta, entityStack, dummyProvider);
                    this.renderEntity(entity, translate.x, translate.y, translate.z, tickDelta, entityStack, dummyProvider);
                }
            }
        }

        
        // Update matrices to shader
        float[] floatBuffer = new float[16];
        matrices.peek().getPositionMatrix().get(floatBuffer);
        WgpuNative.setMatrix(2, floatBuffer);

        floatBuffer = new float[16];
        RenderSystem.getProjectionMatrix().get(floatBuffer);
        WgpuNative.setMatrix(0, floatBuffer);


        if(player != null) {
            WgpuNative.setSectionPos((int)Math.floor(translate.x/16.0),(int)Math.floor(translate.z/16.0));
            MatrixStack stack = new MatrixStack();
            stack.push();
            stack.translate(-(translate.x%16+16)%16, -translate.y, -(translate.z%16+16)%16);

            floatBuffer = new float[16];
            stack.peek().getPositionMatrix().get(floatBuffer);
            WgpuNative.setMatrix(3, floatBuffer); // Terrain transformation matrix
        }

        ci.cancel();
    }

    public void bindSkyData(MatrixStack matrices, Matrix4f projectionMatrix, float tickDelta, Camera camera) {
        Vec3d skyColor = this.world.getSkyColor(this.client.gameRenderer.getCamera().getPos(), tickDelta);
        float skyAngle = this.world.getSkyAngle(tickDelta);
        float skyBrightness = this.world.getSkyBrightness(tickDelta);
        float starShimmer = this.world.method_23787(tickDelta);
        
        //matrices.multiply(RotationAxis.POSITIVE_Y.rotationDegrees(-90.0F));
       // matrices.multiply(RotationAxis.POSITIVE_X.rotationDegrees(this.world.getSkyAngle(tickDelta) * 360.0F));

        WgpuNative.bindSkyData((float) skyColor.getX(), (float) skyColor.getY(), (float) skyColor.getZ(), skyAngle, skyBrightness, starShimmer, this.world.getMoonPhase());
    }

    public void bindRenderEffectsData(float[] fogColorOverride) {
        WgpuNative.bindRenderEffectsData(
            RenderSystem.getShaderFogStart(), 
            RenderSystem.getShaderFogEnd(), 
            RenderSystem.getShaderFogShape().getId(), 
            RenderSystem.getShaderFogColor(),
            RenderSystem.getShaderColor(),
            fogColorOverride == null ? new float[4] : fogColorOverride);
    }


    @Inject(method = "reload", cancellable = true, at = @At("HEAD"))
    public void reload(CallbackInfo ci) {
        WgpuNative.reload(this.client.options.getClampedViewDistance(),this.world.getBottomSectionCoord());
    }
}
