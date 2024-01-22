package dev.birb.wgpu.mixin.world;

import net.minecraft.util.collection.PackedIntegerArray;
import org.spongepowered.asm.mixin.Mixin;

@Mixin(PackedIntegerArray.class)
public class PackedIntegerArrayMixin {
//
//    /**
//     * @author
//     * @reason
//     */
//    @Overwrite
//    private int getStorageIndex(int index) {
//        long l = Integer.toUnsignedLong(this.indexScale);
//        long m = Integer.toUnsignedLong(this.indexOffset);
//        return (int)((long)index * l + m >> 32 >> this.indexShift);
//    }
//
//    /**
//     * @author
//     * @reason
//     */
//    @Overwrite
//    public int swap(int index, int value) {
//        Validate.inclusiveBetween(0L, (long)(this.size - 1), (long)index);
//        Validate.inclusiveBetween(0L, this.maxValue, (long)value);
//        int i = this.getStorageIndex(index);
//        long l = this.data[i];
//        int j = (index - i * this.elementsPerLong) * this.elementBits;
//        int k = (int)(l >> j & this.maxValue);
//        this.data[i] = l & ~(this.maxValue << j) | ((long)value & this.maxValue) << j;
//        return k;
//    }
//
//    /**
//     * @author
//     * @reason
//     */
//    @Overwrite
//    public void set(int index, int value) {
//        Validate.inclusiveBetween(0L, (long)(this.size - 1), (long)index);
//        Validate.inclusiveBetween(0L, this.maxValue, (long)value);
//        int i = this.getStorageIndex(index);
//        long l = this.data[i];
//        int j = (index - i * this.elementsPerLong) * this.elementBits;
//        this.data[i] = l & ~(this.maxValue << j) | ((long)value & this.maxValue) << j;
//    }
//
//    /**
//     * @author
//     * @reason
//     */
//    @Overwrite
//    public int get(int index) {
//        Validate.inclusiveBetween(0L, (long)(this.size - 1), (long)index);
//        int i = this.getStorageIndex(index);
//        long l = this.data[i];
//        int j = (index - i * this.elementsPerLong) * this.elementBits;
//        return (int)(l >> j & this.maxValue);
//    }
//
//    /**
//     * @author
//     * @reason
//     */
//    @Overwrite
//    public void forEach(IntConsumer action) {
//        int i = 0;
//
//        for(long l : this.data) {
//            for(int j = 0; j < this.elementsPerLong; ++j) {
//                action.accept((int)(l & this.maxValue));
//                l >>= this.elementBits;
//                if (++i >= this.size) {
//                    return;
//                }
//            }
//        }
//    }
//
//    /**
//     * @author
//     * @reason
//     */
//    @Overwrite
//    public void writePaletteIndices(int[] out) {
//
//
//        int i = this.data.length;
//        int j = 0;
//
//        for(int k = 0; k < i - 1; ++k) {
//            long l = this.data[k];
//
//            for(int m = 0; m < this.elementsPerLong; ++m) {
//                out[j + m] = (int)(l & this.maxValue);
//                l >>= this.elementBits;
//            }
//
//            j += this.elementsPerLong;
//        }
//
//        int k = this.size - j;
//        if (k > 0) {
//            long l = this.data[i - 1];
//
//            for(int m = 0; m < k; ++m) {
//                out[j + m] = (int)(l & this.maxValue);
//                l >>= this.elementBits;
//            }
//        }
//    }
//
//    /**
//     * @author
//     * @reason
//     */
//    @Overwrite
//    public PaletteStorage copy() {
//        return new PackedIntegerArray(this.elementBits, this.size, (long[])this.data.clone());
//    }
//

}
