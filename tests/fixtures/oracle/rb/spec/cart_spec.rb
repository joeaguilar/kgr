require_relative "../lib/shop"

describe Shop::Cart do
  it "adds" do
    Shop::Cart.new.add(1).total
  end
end
