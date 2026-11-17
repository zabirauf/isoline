package com.example.shop;

import java.util.ArrayList;
import java.util.*;
import static java.lang.Math.max;

/**
 * A shopping cart.
 */
public class Cart {
    private final List<Item> items = new ArrayList<>();

    public Cart() {}

    /** Adds one item. */
    public void add(Item item) {
        items.add(item);
        log("added");
    }

    public void add(String name) {
        add(new Item(name));
    }

    static void log(String msg) {
        System.out.println(msg);
    }

    enum State { OPEN, CLOSED }
}

interface Priced {
    int price();
}
