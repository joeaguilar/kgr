module Shop
  class Cart
    def initialize
      @items = []
    end

    def add(item)
      @items << item
      self
    end

    def total
      @items.sum
    end

    private

    def secret; end
  end
end
